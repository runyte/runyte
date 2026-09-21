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
#[cfg(not(windows))]
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

fn proposal(text: &str, explanation: Vec<Detail>) -> App {
    let mut app = App::new(Config::default(), None).unwrap();
    app.context_ui.surface = Some(Surface::new(
        Kind::Proposal { id: "p".into() },
        "Review terminal text".into(),
        explanation,
        text,
        app.plugins.attachment_generation,
    ));
    app
}

fn present(app: &mut App, frame: u64) {
    app.note_context_frame(80, 22, frame);
    app.note_context_presented(frame);
}

fn press(app: &mut App, code: KeyCode) {
    app.handle_input(InputEvent::Key(KeyStroke::plain(code)))
        .unwrap();
}

fn realistic_explanation() -> Vec<Detail> {
    vec![
        Detail::value("Agent", visible("claude")),
        Detail::value(
            "Terminal",
            format!("{} (#1)", label("\u{25d1} Runyte reconnection ENOTDIR")),
        ),
        Detail::value("Workspace", label("/home/user/code/runyte")),
        Detail::value(
            "Reason",
            format!("{} (unverified)", label("Test of terminal text proposal")),
        ),
        Detail::note("Inserted at the input position without Enter; it may still act."),
        Detail::excerpt("Output", &label(&"─".repeat(126))),
        Detail::excerpt("", &label("> check the proposal status")),
        Detail::excerpt("", &label("  auto mode on")),
        Detail::note("Recent output does not show whether the input line is empty."),
    ]
}

#[test]
fn short_proposal_is_one_page_that_leads_with_the_proposed_text() {
    let mut app = proposal("Hello to myself", realistic_explanation());
    present(&mut app, 1);
    let overlay = app.context_overlay().unwrap();
    let first = &overlay.rows[0];
    assert_eq!(first.label, format!("{:<11}Hello·to·myself", "Text"));
    assert_eq!(first.muted, (0..11).collect::<Vec<_>>());
    assert_eq!(first.emphasis, (11..26).collect::<Vec<_>>());
    assert!(
        overlay
            .rows
            .iter()
            .all(|row| !row.label.starts_with("Page")),
        "{:#?}",
        overlay.rows
    );
    assert!(
        overlay.rows.iter().any(|row| row
            .label
            .contains("\u{25d1} Runyte reconnection ENOTDIR (#1)")),
        "labels keep printable Unicode and plain spaces"
    );
    assert!(
        overlay
            .rows
            .iter()
            .all(|row| row.label.chars().count() <= 76)
    );
    let rows = overlay.rows.len();
    assert_eq!(overlay.rows[rows - 2].label, "Reject");
    assert_eq!(overlay.rows[rows - 1].label, "Insert text (no Enter)");
    assert_eq!(overlay.selected, Some(rows - 2), "Reject starts selected");
    assert!(overlay.message.is_none());

    press(&mut app, KeyCode::Tab);
    assert_eq!(app.context_overlay().unwrap().selected, Some(rows - 1));
    press(&mut app, KeyCode::Enter);
    assert!(matches!(
        app.context_ui.decision,
        Some(Decision::Proposal { accepted: true, .. })
    ));
}

#[test]
fn refused_insert_names_the_unread_page_until_every_page_is_shown() {
    let mut app = proposal(&"x".repeat(64 * 14), vec![]);
    present(&mut app, 1);
    press(&mut app, KeyCode::Tab);
    press(&mut app, KeyCode::Enter);
    assert!(app.context_ui.decision.is_none());
    let overlay = app.context_overlay().unwrap();
    assert_eq!(
        overlay.message.as_deref(),
        Some("Read page 2/2 first: press j until the last page has been shown.")
    );
    assert!(
        overlay
            .rows
            .iter()
            .any(|row| row.label.starts_with("Page 1/2"))
    );
    assert_eq!(overlay.actions[0].key_hint, "j/k");

    press(&mut app, KeyCode::Char('j'));
    present(&mut app, 2);
    let overlay = app.context_overlay().unwrap();
    assert!(overlay.message.is_none());
    assert_eq!(
        overlay.rows[0].label,
        format!("{:<11}{}", "", "x".repeat(64))
    );
    press(&mut app, KeyCode::Enter);
    assert!(matches!(
        app.context_ui.decision,
        Some(Decision::Proposal { accepted: true, .. })
    ));
}

#[test]
fn labels_stay_readable_but_nothing_in_them_can_hide() {
    assert_eq!(label("◐ build · 界"), "◐ build · 界");
    assert_eq!(
        label("a\u{202e}b\u{200b}c\u{a0}d\te\\"),
        "a\\u{202e}b\\u{200b}c\\u{a0}d\\u{9}e\\\\"
    );
    assert_eq!(
        Detail::excerpt("Output", &"y".repeat(70)).value,
        format!("{}…", "y".repeat(63))
    );
}

#[test]
fn prose_wraps_between_words_and_proposed_text_anywhere() {
    let app = proposal(
        "ab cd",
        vec![Detail::note(format!(
            "{} tail",
            "word ".repeat(13).trim_end()
        ))],
    );
    let lines = &app.context_ui.surface.as_ref().unwrap().lines;
    assert_eq!(lines[0].value, "ab·cd");
    let prose = lines
        .iter()
        .filter(|line| line.value.starts_with("word") || line.value == "tail")
        .map(|line| line.value.as_str())
        .collect::<Vec<_>>();
    assert_eq!(prose, [&"word ".repeat(13)[..64], "tail"]);
}

#[test]
fn arrows_move_between_the_choices_without_paging() {
    let mut app = proposal(&"x".repeat(64 * 14), vec![]);
    present(&mut app, 1);
    let rows = app.context_overlay().unwrap().rows.len();
    press(&mut app, KeyCode::Down);
    press(&mut app, KeyCode::Down);
    assert_eq!(app.context_overlay().unwrap().selected, Some(rows - 1));
    press(&mut app, KeyCode::Up);
    assert_eq!(app.context_overlay().unwrap().selected, Some(rows - 2));
    assert_eq!(app.context_ui.surface.as_ref().unwrap().page, 0);
    press(&mut app, KeyCode::Enter);
    assert!(matches!(
        app.context_ui.decision,
        Some(Decision::Proposal {
            accepted: false,
            ..
        })
    ));
}
