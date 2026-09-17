// SPDX-License-Identifier: MPL-2.0

use super::*;
use crate::{config::Config, input::KeyStroke};

fn ready() -> App {
    let mut app = App::new(Config::default(), None).unwrap();
    app.context_ui.surface = Some(Surface::new(
        Kind::Proposal { id: "p".into() },
        "Review terminal text".into(),
        vec![],
        "literal",
        app.plugins.attachment_generation,
    ));
    app.note_context_frame(80, 22, 1);
    app.note_context_presented(1);
    app.context_ui.surface.as_mut().unwrap().accept_selected = true;
    app
}

#[test]
fn native_command_prompt_requests_named_and_default_context_identity() {
    for (command, identity) in [
        ("context-access codex", "codex"),
        ("context-access", "agent"),
    ] {
        let mut app = App::new(Config::default(), None).unwrap();
        for character in format!(":{command}").chars() {
            app.handle_input(InputEvent::Key(KeyStroke::plain(KeyCode::Char(character))))
                .unwrap();
        }
        app.handle_input(InputEvent::Key(KeyStroke::plain(KeyCode::Enter)))
            .unwrap();
        assert_eq!(app.context_ui.requested_identity.as_deref(), Some(identity));
        assert_ne!(app.mode, crate::command::Mode::Command);
        assert!(app.context_ui.decision.is_none());
    }
}

#[test]
fn macro_replay_paste_and_modified_keys_cannot_approve() {
    let mut app = ready();
    app.handle_replayed_input(InputEvent::Key(KeyStroke::plain(KeyCode::Enter)))
        .unwrap();
    assert!(app.context_ui.decision.is_none());
    assert!(app.context_overlay_active());
    app.handle_input(InputEvent::Text("\r\n".into())).unwrap();
    app.handle_input(InputEvent::Key(KeyStroke::ctrl('m')))
        .unwrap();
    assert!(app.context_ui.decision.is_none());
    assert!(app.context_overlay_active());
    app.recording_macro = Some('a');
    app.handle_input(InputEvent::Key(KeyStroke::plain(KeyCode::Enter)))
        .unwrap();
    assert!(app.context_ui.decision.is_none());
    app.request_context_access(None);
    assert!(app.context_ui.requested_identity.is_none());
}

#[test]
fn small_or_stale_frontend_cannot_approve_and_literal_spelling_is_unambiguous() {
    let mut app = ready();
    app.note_context_frame(40, 12, 2);
    app.note_context_presented(2);
    app.handle_input(InputEvent::Key(KeyStroke::plain(KeyCode::Enter)))
        .unwrap();
    assert!(app.context_ui.decision.is_none());
    assert!(app.context_overlay_active());
    app.plugins.attachment_generation += 1;
    app.handle_input(InputEvent::Key(KeyStroke::plain(KeyCode::Enter)))
        .unwrap();
    assert!(app.context_ui.decision.is_none());
    assert!(!app.context_overlay_active());
    assert_eq!(visible(" \\n界\u{202e}"), "·\\\\n\\u{754c}\\u{202e}");
    assert_ne!(visible("\\u{202e}"), visible("\u{202e}"));
}
