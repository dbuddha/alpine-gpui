use alpine_platform_macos::{ImeEvent, Modifiers};
use alpine_text::{ByteOffset, Selection};

use super::*;
use crate::settings::{
    ChordPrefix, KEY_DELETE_BACKWARD, KEY_ESCAPE, KEY_G, KEY_I, KEY_K, KEY_LEFT, KEY_RETURN, KEY_S,
};
use crate::tests::TestTextSystem;

#[test]
fn unedited_buffer_can_stamp_an_lsp_request() -> Result<(), Box<dyn std::error::Error>> {
    let app = EditorApp::new(TestTextSystem)?;
    assert!(app.runtime_document_revision >= 1);
    assert!(app.language_identity().request_stamp().is_some());
    Ok(())
}

#[test]
fn ctrl_g_opens_go_to_line_and_return_jumps() -> Result<(), Box<dyn std::error::Error>> {
    let mut app = EditorApp::new(TestTextSystem)?;
    let control = Modifiers::from_bits(Modifiers::CONTROL);
    assert!(app.handle_key(KEY_G, control).visual_changed);
    assert!(app.go_to_line.is_open());
    assert_eq!(app.go_to_line.query(), "1");
    let snapshot = app.accessibility_snapshot()?;
    assert!(
        snapshot
            .nodes()
            .iter()
            .any(|node| node.name() == "Go to line" && node.is_focused())
    );

    assert!(
        app.handle_ime(&ImeEvent::Committed("3".into()))
            .visual_changed
    );
    assert_eq!(app.go_to_line.query(), "3");
    assert!(
        app.handle_key(KEY_RETURN, Modifiers::default())
            .visual_changed
    );
    assert!(!app.go_to_line.is_open());
    let expected = app.buffer().snapshot().line_byte_range(2)?.start;
    assert_eq!(app.selection, Selection::caret(ByteOffset::new(expected)));
    Ok(())
}

#[test]
fn ctrl_g_rejects_zero_and_escape_closes() -> Result<(), Box<dyn std::error::Error>> {
    let mut app = EditorApp::new(TestTextSystem)?;
    app.handle_key(KEY_G, Modifiers::from_bits(Modifiers::CONTROL));
    app.handle_ime(&ImeEvent::Committed("0".into()));
    let before = app.selection;
    assert!(
        app.handle_key(KEY_RETURN, Modifiers::default())
            .visual_changed
    );
    assert!(app.go_to_line.is_open());
    assert_eq!(app.selection, before);
    assert!(
        app.local_status
            .as_ref()
            .is_some_and(|status| status.message().contains("1-based"))
    );
    assert!(
        app.handle_key(KEY_ESCAPE, Modifiers::default())
            .visual_changed
    );
    assert!(!app.go_to_line.is_open());
    Ok(())
}

#[test]
fn cmd_k_cmd_i_resolves_hover_chord_and_unmatched_second_key_falls_through()
-> Result<(), Box<dyn std::error::Error>> {
    let mut app = EditorApp::new(TestTextSystem)?;
    let command = Modifiers::from_bits(Modifiers::COMMAND);
    assert!(app.handle_key(KEY_K, command).visual_changed);
    assert_eq!(app.pending_chord, Some(ChordPrefix::CommandK));
    assert!(app.handle_key(KEY_I, command).visual_changed);
    assert_eq!(app.pending_chord, None);

    assert!(app.handle_key(KEY_K, command).visual_changed);
    assert_eq!(app.pending_chord, Some(ChordPrefix::CommandK));
    app.handle_key(KEY_S, command);
    assert_eq!(app.pending_chord, None);
    assert!(app.handle_key(KEY_K, command).visual_changed);
    assert!(
        app.handle_key(KEY_ESCAPE, Modifiers::default())
            .visual_changed
    );
    assert_eq!(app.pending_chord, None);
    Ok(())
}

#[test]
fn cmd_k_cmd_i_does_not_fire_when_shift_makes_the_format_binding()
-> Result<(), Box<dyn std::error::Error>> {
    let mut app = EditorApp::new(TestTextSystem)?;
    let command = Modifiers::from_bits(Modifiers::COMMAND);
    let command_shift = Modifiers::from_bits(Modifiers::COMMAND | Modifiers::SHIFT);
    app.handle_key(KEY_K, command);
    app.handle_key(KEY_I, command_shift);
    assert_eq!(app.pending_chord, None);
    Ok(())
}

#[test]
fn go_to_line_field_edits_with_delete_and_arrows() -> Result<(), Box<dyn std::error::Error>> {
    let mut app = EditorApp::new(TestTextSystem)?;
    app.handle_key(KEY_G, Modifiers::from_bits(Modifiers::CONTROL));
    app.handle_ime(&ImeEvent::Committed("12".into()));
    assert_eq!(app.go_to_line.query(), "12");
    assert!(
        app.handle_key(KEY_DELETE_BACKWARD, Modifiers::default())
            .visual_changed
    );
    assert_eq!(app.go_to_line.query(), "1");
    assert!(
        app.handle_key(KEY_LEFT, Modifiers::default())
            .visual_changed
    );
    app.handle_ime(&ImeEvent::Committed("9".into()));
    assert_eq!(app.go_to_line.query(), "91");
    Ok(())
}
