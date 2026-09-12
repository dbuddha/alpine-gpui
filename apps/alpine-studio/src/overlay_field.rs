//! Focus-aware access to the six private overlay field owners.

use alpine_platform_macos::{AccessibilityNodeId, Modifiers};
use alpine_text::Selection;

use crate::field_edit::{EditError, FieldEdit, Prepared};
use crate::settings::{
    KEY_A, KEY_DELETE_BACKWARD, KEY_DELETE_FORWARD, KEY_END, KEY_HOME, KEY_LEFT, KEY_RIGHT, KEY_Z,
};
use crate::{EventEffect, StudioApp};
use alpine_text::ByteOffset;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Owner {
    Find,
    Palette,
    QuickOpen,
    ProjectSearch,
    Symbols,
    Rename,
}

impl Owner {
    pub(super) fn active(app: &StudioApp) -> Option<Self> {
        let node = crate::accessibility::focus_owner(app)?;
        [
            Self::Find,
            Self::Palette,
            Self::QuickOpen,
            Self::ProjectSearch,
            Self::Symbols,
            Self::Rename,
        ]
        .into_iter()
        .find(|owner| owner.node(app) == node && owner.read(app).is_some())
    }

    pub(super) fn node(self, app: &StudioApp) -> AccessibilityNodeId {
        use crate::accessibility as ax;
        match self {
            Self::Find => ax::find_node(app),
            Self::Palette => ax::COMMAND_PALETTE_FIELD,
            Self::QuickOpen => ax::QUICK_OPEN_FIELD,
            Self::ProjectSearch => ax::PROJECT_SEARCH_FIELD,
            Self::Symbols => ax::SYMBOL_FIELD,
            Self::Rename => ax::RENAME_FIELD,
        }
    }

    pub(super) fn read(self, app: &StudioApp) -> Option<(&str, &FieldEdit)> {
        Some(match self {
            Self::Find => (app.find.field_text(), app.find.edit()),
            Self::Palette => (app.command_palette.query(), &app.command_palette.edit),
            Self::QuickOpen => (app.quick_open.query(), &app.quick_open.edit),
            Self::ProjectSearch => (app.project_search.query(), &app.project_search.edit),
            Self::Symbols => {
                let picker = app
                    .rust_diagnostics
                    .symbol_picker(app.language_identity())?;
                (picker.query(), &picker.edit)
            }
            Self::Rename => return app.workspace_edits.input(),
        })
    }

    pub(super) fn parts(self, app: &mut StudioApp) -> Option<(&str, &mut FieldEdit)> {
        Some(match self {
            Self::Find => app.find.edit_parts(),
            Self::Palette => app.command_palette.edit_parts(),
            Self::QuickOpen => app.quick_open.edit_parts(),
            Self::ProjectSearch => app.project_search.edit_parts(),
            Self::Symbols => {
                let identity = app.language_identity();
                app.rust_diagnostics
                    .symbol_picker_mut(identity)?
                    .edit_parts()
            }
            Self::Rename => return app.workspace_edits.input_mut(),
        })
    }

    pub(super) fn prefix(self, app: &StudioApp) -> &'static str {
        match self {
            Self::Find => app.find.display_prefix(),
            Self::Palette => "> ",
            Self::QuickOpen => "Quick Open: ",
            Self::ProjectSearch => "Project Search: ",
            Self::Symbols => "",
            Self::Rename => "Rename Rust symbol: ",
        }
    }

    pub(super) const fn limit(self) -> usize {
        match self {
            Self::Find => crate::find::MAX_QUERY_BYTES,
            Self::Palette => crate::commands::MAX_QUERY_BYTES,
            Self::QuickOpen => crate::quick_open::MAX_QUERY_BYTES,
            Self::ProjectSearch => crate::project_search::MAX_QUERY_BYTES,
            Self::Symbols => crate::rust_symbols::MAX_SYMBOL_QUERY_BYTES,
            Self::Rename => crate::rust_workspace_ui::MAX_RENAME_INPUT_BYTES,
        }
    }

    pub(super) fn set_selection(
        self,
        app: &mut StudioApp,
        selection: Selection,
    ) -> Result<bool, EditError> {
        let (value, edit) = self.parts(app).ok_or(EditError::InvalidSelection)?;
        let changed = edit.set_selection(value, selection)?;
        if self == Self::Rename {
            app.workspace_edits
                .rebuild_rename_lines()
                .map_err(|_| EditError::AllocationFailed)?;
        }
        Ok(changed)
    }

    pub(super) fn apply(self, app: &mut StudioApp, prepared: Prepared) -> EventEffect {
        let effect = match self {
            Self::Find => match app.find.apply_edit(prepared) {
                Ok(changed) => {
                    app.find_needs_search |= changed;
                    EventEffect::visual()
                }
                Err(error) => app.record_find_error(&error),
            },
            Self::Palette => {
                let context = app.command_context();
                match app.command_palette.apply_edit(prepared, context) {
                    Ok(_) => EventEffect::visual(),
                    Err(error) => app.record_command_palette_error(&error),
                }
            }
            Self::QuickOpen => match app.quick_open.apply_edit(prepared) {
                Ok(_) => EventEffect::visual(),
                Err(error) => app.record_quick_open_error(&error),
            },
            Self::ProjectSearch => match app.project_search.apply_edit(prepared) {
                Ok(_) => EventEffect::visual(),
                Err(error) => app.record_project_search_error(&error),
            },
            Self::Symbols => {
                let identity = app.language_identity();
                let effect = app.rust_diagnostics.apply_symbol_edit(identity, prepared);
                effect
                    .visual_changed
                    .then(EventEffect::visual)
                    .unwrap_or_default()
                    .merge(EventEffect::visual())
            }
            Self::Rename => match app.workspace_edits.apply_edit(prepared) {
                Ok(_) => EventEffect::visual(),
                Err(error) => app.record_workspace_edit_panel_error(error),
            },
        };
        if let Some((_, edit)) = self.parts(app) {
            edit.cancel_composition();
        }
        effect
    }

    pub(super) fn commit(self, app: &mut StudioApp, text: &str, caret: usize) -> EventEffect {
        let prepared = (|| {
            if matches!(self, Self::Symbols | Self::Rename) && text.chars().any(char::is_control) {
                return Err(EditError::InvalidSelection);
            }
            let (value, edit) = self.parts(app).ok_or(EditError::InvalidSelection)?;
            edit.prepare(value, text, caret, self.limit())
        })();
        match prepared {
            Ok(prepared) => self.apply(app, prepared),
            Err(error) => self.reject(app, error),
        }
    }

    pub(super) fn reject(self, app: &mut StudioApp, error: EditError) -> EventEffect {
        if let Some((_, edit)) = self.parts(app) {
            edit.cancel_composition();
        }
        if self == Self::Rename {
            let _ = app.workspace_edits.rebuild_rename_lines();
        }
        match self {
            Self::Find => app.record_find_error(&error.into()),
            Self::Palette => app.record_command_palette_error(&error.into()),
            Self::QuickOpen => app.record_quick_open_error(&error.into()),
            Self::ProjectSearch => app.record_project_search_error(&error.into()),
            Self::Symbols => {
                app.input_failures = app.input_failures.saturating_add(1);
                let effect = app.rust_diagnostics.record_symbol_error(error.into());
                effect
                    .visual_changed
                    .then(EventEffect::visual)
                    .unwrap_or_default()
            }
            Self::Rename => app.record_workspace_edit_panel_error(error.into()),
        }
        .merge(EventEffect::visual())
    }

    pub(super) fn key(
        self,
        app: &mut StudioApp,
        key: u16,
        modifiers: Modifiers,
    ) -> Option<EventEffect> {
        let command = modifiers.contains(Modifiers::COMMAND);
        let shift = modifiers.contains(Modifiers::SHIFT);
        if matches!(key, KEY_LEFT | KEY_RIGHT | KEY_HOME | KEY_END) || (command && key == KEY_A) {
            let result = (|| {
                let (value, edit) = self.parts(app).ok_or(EditError::InvalidSelection)?;
                if key == KEY_A {
                    edit.set_selection(
                        value,
                        Selection::new(ByteOffset::new(0), ByteOffset::new(value.len())),
                    )
                } else {
                    edit.move_caret(
                        value,
                        matches!(key, KEY_RIGHT | KEY_END),
                        shift,
                        command || matches!(key, KEY_HOME | KEY_END),
                    )
                }
            })();
            if self == Self::Rename {
                let _ = app.workspace_edits.rebuild_rename_lines();
            }
            return Some(match result {
                Ok(changed) => changed.then(EventEffect::visual).unwrap_or_default(),
                Err(error) => self.reject(app, error),
            });
        }
        let history = command && key == KEY_Z;
        if history || (!command && matches!(key, KEY_DELETE_BACKWARD | KEY_DELETE_FORWARD)) {
            let result = (|| {
                let (value, edit) = self.parts(app).ok_or(EditError::InvalidSelection)?;
                if history {
                    edit.prepare_history(value, shift)
                } else {
                    edit.prepare_delete(value, key == KEY_DELETE_FORWARD, self.limit())
                        .map(Some)
                }
            })();
            return Some(match result {
                Ok(Some(prepared)) => self.apply(app, prepared),
                Ok(None) => EventEffect::default(),
                Err(error) => self.reject(app, error),
            });
        }
        None
    }
}

/// Clipboard completion is synchronous in the native bridge. Keep a captured
/// owner anyway so injected/delayed responses cannot edit a different field.
pub(super) struct PendingClipboard {
    owner: Owner,
    node: AccessibilityNodeId,
    document: u64,
    epoch: alpine_platform_macos::InputEpoch,
    selection: Selection,
    value: String,
    operation: alpine_platform_macos::ClipboardOperation,
    pub(super) valid: bool,
}

impl PendingClipboard {
    fn capture(
        app: &StudioApp,
        owner: Owner,
        operation: alpine_platform_macos::ClipboardOperation,
    ) -> Option<Self> {
        let (value, edit) = owner.read(app)?;
        Some(Self {
            owner,
            node: owner.node(app),
            document: app.runtime_document_revision,
            epoch: app.input_epoch,
            selection: edit.selection(value),
            value: value.to_owned(),
            operation,
            valid: true,
        })
    }

    fn current(&self, app: &StudioApp) -> bool {
        self.valid
            && Owner::active(app) == Some(self.owner)
            && self.node == self.owner.node(app)
            && self.document == app.runtime_document_revision
            && self.epoch == app.input_epoch
            && self.owner.read(app).is_some_and(|(value, edit)| {
                value == self.value
                    && edit.selection(value) == self.selection
                    && !edit.is_composing()
            })
    }
}

impl StudioApp {
    pub(super) fn begin_field_clipboard(
        &mut self,
        owner: Owner,
        operation: alpine_platform_macos::ClipboardOperation,
    ) -> crate::StudioTransition {
        use alpine_platform_macos::{ClipboardOperation, ClipboardText, ClipboardWrite};
        self.pending_cut = None;
        self.pending_field_clipboard = None;
        let Some((value, edit)) = owner.read(self) else {
            return crate::StudioTransition::default();
        };
        // Native composition owns provisional text. Clipboard operations use only
        // an explicit committed selection after composition is resolved.
        if edit.is_composing() {
            return crate::StudioTransition::default();
        }
        if operation == ClipboardOperation::Paste {
            self.pending_field_clipboard = PendingClipboard::capture(self, owner, operation);
            return crate::StudioTransition::default();
        }
        let range = edit.selection(value).range();
        if range.is_empty() {
            return crate::StudioTransition::default();
        }
        let text = value[range].to_owned();
        let write = ClipboardText::new(text).and_then(|text| ClipboardWrite::new(operation, text));
        match write {
            Ok(write) => {
                if operation == ClipboardOperation::Cut {
                    self.pending_field_clipboard =
                        PendingClipboard::capture(self, owner, operation);
                }
                crate::StudioTransition {
                    effect: self.clear_clipboard_status(),
                    clipboard_write: Some(write),
                    cancel_close: false,
                }
            }
            Err(error) => crate::StudioTransition::effect(self.record_clipboard_error(error)),
        }
    }

    pub(super) fn complete_field_clipboard(
        &mut self,
        event: &alpine_platform_macos::ClipboardEvent,
    ) -> Option<EventEffect> {
        use alpine_platform_macos::ClipboardEvent;
        if matches!(event, ClipboardEvent::CopyCompleted(_)) {
            return None;
        }
        let pending = self.pending_field_clipboard.take()?;
        if pending.operation != event.operation() || !pending.current(self) {
            return Some(self.record_clipboard_protocol_failure(
                "Clipboard completion no longer owns the original field selection.",
            ));
        }
        Some(match event {
            ClipboardEvent::CutCompleted(Ok(())) => pending.owner.commit(self, "", 0),
            ClipboardEvent::PasteCompleted(Ok(text)) => {
                pending
                    .owner
                    .commit(self, text.as_str(), text.as_str().len())
            }
            ClipboardEvent::CutCompleted(Err(error))
            | ClipboardEvent::PasteCompleted(Err(error)) => self.record_clipboard_error(*error),
            ClipboardEvent::CopyCompleted(_) => unreachable!(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::TestTextSystem;
    use alpine_platform_macos::{ClipboardEvent, ClipboardOperation, ClipboardText, ImeEvent};

    fn open(app: &mut StudioApp, owner: Owner) -> Result<(), Box<dyn std::error::Error>> {
        match owner {
            Owner::Find => {
                app.find.open(false);
            }
            Owner::Palette => {
                app.command_palette.open(app.command_context())?;
            }
            Owner::QuickOpen => {
                app.quick_open.open(1)?;
            }
            Owner::ProjectSearch => {
                app.project_search.open(1)?;
            }
            Owner::Rename => {
                app.workspace_edits.open_rename()?;
            }
            Owner::Symbols => unreachable!(),
        }
        Ok(())
    }

    #[test]
    fn fields_replace_unicode_selection_preserve_document_and_undo()
    -> Result<(), Box<dyn std::error::Error>> {
        for owner in [
            Owner::Find,
            Owner::Palette,
            Owner::QuickOpen,
            Owner::ProjectSearch,
            Owner::Rename,
        ] {
            let mut app = StudioApp::new(TestTextSystem)?;
            let original = app.buffer().snapshot().text();
            open(&mut app, owner)?;
            assert_eq!(Owner::active(&app), Some(owner));
            assert!(
                app.handle_ime(&ImeEvent::Committed("a😀z".into()))
                    .visual_changed
            );
            owner.set_selection(
                &mut app,
                Selection::new(ByteOffset::new(1), ByteOffset::new(5)),
            )?;
            app.handle_ime(&ImeEvent::Started);
            app.handle_ime(&ImeEvent::Updated {
                text: "日本".into(),
                selected_start_utf16: 1,
                selected_length_utf16: 1,
            });
            let (value, edit) = owner.read(&app).ok_or("field")?;
            assert_eq!(value, "a😀z");
            assert_eq!(edit.projected_value(value)?, "a日本z");
            app.handle_ime(&ImeEvent::CommittedWithCaret {
                text: "猫犬".into(),
                caret_utf16: 1,
            });
            let (value, edit) = owner.read(&app).ok_or("field")?;
            assert_eq!(value, "a猫犬z");
            assert_eq!(edit.selection(value).head().get(), 4);
            app.handle_key(KEY_Z, Modifiers::from_bits(Modifiers::COMMAND));
            assert_eq!(owner.read(&app).ok_or("field")?.0, "a😀z");
            app.handle_key(
                KEY_Z,
                Modifiers::from_bits(Modifiers::COMMAND | Modifiers::SHIFT),
            );
            assert_eq!(owner.read(&app).ok_or("field")?.0, "a猫犬z");
            assert_eq!(app.buffer().snapshot().text(), original);
        }
        Ok(())
    }

    #[test]
    fn clipboard_cut_and_paste_edit_only_captured_field() -> Result<(), Box<dyn std::error::Error>>
    {
        let mut app = StudioApp::new(TestTextSystem)?;
        let original = app.buffer().snapshot().text();
        open(&mut app, Owner::Find)?;
        Owner::Find.commit(&mut app, "a😀z", 6);
        Owner::Find.set_selection(
            &mut app,
            Selection::new(ByteOffset::new(1), ByteOffset::new(5)),
        )?;
        let transition = app.begin_field_clipboard(Owner::Find, ClipboardOperation::Cut);
        assert!(transition.clipboard_write.is_some());
        assert_eq!(app.find.field_text(), "a😀z");
        app.handle_clipboard_completion(&ClipboardEvent::CutCompleted(Ok(())));
        assert_eq!(app.find.field_text(), "az");
        app.begin_field_clipboard(Owner::Find, ClipboardOperation::Paste);
        app.handle_clipboard_completion(&ClipboardEvent::PasteCompleted(Ok(ClipboardText::new(
            "猫".to_owned(),
        )?)));
        assert_eq!(app.find.field_text(), "a猫z");
        assert_eq!(app.buffer().snapshot().text(), original);
        Ok(())
    }

    #[test]
    fn stale_cut_cannot_delete_another_field_or_document() -> Result<(), Box<dyn std::error::Error>>
    {
        let mut app = StudioApp::new(TestTextSystem)?;
        let original = app.buffer().snapshot().text();
        open(&mut app, Owner::Find)?;
        Owner::Find.commit(&mut app, "query", 5);
        app.find.select_all();
        app.begin_field_clipboard(Owner::Find, ClipboardOperation::Cut);
        app.find.open(true);
        Owner::Find.commit(&mut app, "replace", 7);
        app.handle_clipboard_completion(&ClipboardEvent::CutCompleted(Ok(())));
        assert_eq!(app.find.query(), "query");
        assert_eq!(app.find.replacement(), "replace");
        app.find.select_all();
        app.begin_field_clipboard(Owner::Find, ClipboardOperation::Cut);
        app.find.close();
        app.handle_clipboard_completion(&ClipboardEvent::CutCompleted(Ok(())));
        assert_eq!(app.buffer().snapshot().text(), original);
        assert_eq!(app.find.replacement(), "replace");
        Ok(())
    }

    #[test]
    fn field_drag_cannot_start_in_document_or_survive_focus_and_owner_changes()
    -> Result<(), Box<dyn std::error::Error>> {
        use alpine_core::Point;
        use alpine_platform_macos::{PointerAction, PointerButton};
        let mut app = StudioApp::new(TestTextSystem)?;
        let document_selection = app.selection;
        open(&mut app, Owner::Rename)?;
        Owner::Rename.commit(&mut app, "rename", 6);
        let field_selection = Owner::Rename
            .read(&app)
            .ok_or("field")?
            .1
            .selection("rename");
        let outside = Point::new(1.0, 200.0).ok_or("point")?;
        let bounds = crate::find_input::bounds_for(&app, Owner::Rename)?;
        let inside =
            Point::new(bounds.origin().x() + 20.0, bounds.origin().y() + 5.0).ok_or("point")?;
        app.handle_pointer(
            PointerAction::Down,
            outside,
            PointerButton::Primary,
            Modifiers::from_bits(0),
        );
        app.handle_pointer(
            PointerAction::Moved,
            inside,
            PointerButton::None,
            Modifiers::from_bits(0),
        );
        assert_eq!(app.selection, document_selection);
        assert_eq!(
            Owner::Rename
                .read(&app)
                .ok_or("field")?
                .1
                .selection("rename"),
            field_selection
        );
        assert!(app.field_pointer_owner.is_none());
        app.workspace_edits.cancel();
        open(&mut app, Owner::Palette)?;
        app.field_pointer_owner = Some(Owner::Palette.node(&app));
        let next_epoch = app.input_epoch.checked_next().ok_or("epoch")?;
        app.handle_focus(next_epoch, false);
        assert!(app.field_pointer_owner.is_none());
        app.handle_focus(next_epoch, true);
        app.field_pointer_owner = Some(Owner::Palette.node(&app));
        app.handle_key(crate::settings::KEY_ESCAPE, Modifiers::from_bits(0));
        assert!(app.field_pointer_owner.is_none());
        open(&mut app, Owner::Find)?;
        app.field_pointer_owner = Some(Owner::Find.node(&app));
        app.find.open(true);
        app.handle_pointer(
            PointerAction::Moved,
            outside,
            PointerButton::None,
            Modifiers::from_bits(0),
        );
        assert!(app.field_pointer_owner.is_none());
        assert_eq!(app.selection, document_selection);
        Ok(())
    }

    #[test]
    fn failed_cut_and_stale_paste_preserve_field_and_document()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut app = StudioApp::new(TestTextSystem)?;
        let document = app.buffer().snapshot().text();
        open(&mut app, Owner::Find)?;
        Owner::Find.commit(&mut app, "query", 5);
        app.find.select_all();
        app.begin_field_clipboard(Owner::Find, ClipboardOperation::Cut);
        app.handle_clipboard_completion(&ClipboardEvent::CutCompleted(Err(
            alpine_platform_macos::ClipboardError::WriteRejected,
        )));
        assert_eq!(app.find.query(), "query");
        app.begin_field_clipboard(Owner::Find, ClipboardOperation::Paste);
        app.find.open(true);
        Owner::Find.commit(&mut app, "replacement", 11);
        app.handle_clipboard_completion(&ClipboardEvent::PasteCompleted(Ok(ClipboardText::new(
            "stale".to_owned(),
        )?)));
        assert_eq!(app.find.query(), "query");
        assert_eq!(app.find.replacement(), "replacement");
        assert_eq!(app.buffer().snapshot().text(), document);
        Ok(())
    }
}
