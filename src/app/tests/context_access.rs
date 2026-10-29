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
        ("context-access", ""),
        ("mcp codex", "codex"),
        ("mcp", ""),
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
fn repeated_enter_cannot_approve_proposal_but_repeat_navigation_remains_live() {
    let mut app = ready();
    app.context_ui.surface.as_mut().unwrap().accept_selected = false;
    app.handle_repeated_input(InputEvent::Key(KeyStroke::plain(KeyCode::Down)))
        .unwrap();
    assert!(app.context_ui.surface.as_ref().unwrap().accept_selected);
    app.handle_repeated_input(InputEvent::Key(KeyStroke::plain(KeyCode::Up)))
        .unwrap();
    assert!(!app.context_ui.surface.as_ref().unwrap().accept_selected);
    app.handle_repeated_input(InputEvent::Key(KeyStroke::plain(KeyCode::Down)))
        .unwrap();
    app.handle_repeated_input(InputEvent::Key(KeyStroke::plain(KeyCode::Enter)))
        .unwrap();
    assert!(app.context_ui.decision.is_none());
    assert!(app.context_overlay_active());
    app.handle_input(InputEvent::Key(KeyStroke::plain(KeyCode::Enter)))
        .unwrap();
    assert!(matches!(
        app.context_ui.decision,
        Some(Decision::Proposal { accepted: true, .. })
    ));

    let mut long = proposal(&"x".repeat(76 * 12), vec![]);
    present(&mut long, 1);
    long.handle_repeated_input(InputEvent::Key(KeyStroke::ctrl('d')))
        .unwrap();
    assert_eq!(long.context_ui.surface.as_ref().unwrap().top, 1);
    long.handle_repeated_input(InputEvent::Key(KeyStroke::ctrl('u')))
        .unwrap();
    assert_eq!(long.context_ui.surface.as_ref().unwrap().top, 0);
}

#[test]
fn repeated_enter_cannot_approve_context_grant() {
    let mut app = App::new(Config::default(), None).unwrap();
    app.context_ui.surface = Some(Surface::new(
        Kind::Grant {
            identity: "agent".into(),
            scopes: std::collections::BTreeSet::from([Scope::TerminalRead]),
            remember: false,
        },
        "Review context access".into(),
        vec![],
        "",
        app.plugins.attachment_generation,
    ));
    present(&mut app, 1);
    app.handle_repeated_input(InputEvent::Key(KeyStroke::plain(KeyCode::Down)))
        .unwrap();
    app.handle_repeated_input(InputEvent::Key(KeyStroke::plain(KeyCode::Enter)))
        .unwrap();
    assert!(app.context_ui.decision.is_none());
    assert!(app.context_overlay_active());
    app.handle_input(InputEvent::Key(KeyStroke::plain(KeyCode::Enter)))
        .unwrap();
    assert!(matches!(
        app.context_ui.decision,
        Some(Decision::Grant { .. })
    ));
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
    let first = &overlay.rows[2];
    assert_eq!(first.label, "Hello·to·myself");
    assert!(first.muted.is_empty());
    assert_eq!(first.emphasis, (0..15).collect::<Vec<_>>());
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
fn refused_insert_names_the_unread_line_until_every_line_is_shown() {
    // Twelve rows of text and the note under it: one line more than the
    // smallest approving area shows.
    let mut app = proposal(&"x".repeat(76 * 12), vec![]);
    present(&mut app, 1);
    press(&mut app, KeyCode::Tab);
    press(&mut app, KeyCode::Enter);
    assert!(app.context_ui.decision.is_none());
    let overlay = app.context_overlay().unwrap();
    assert_eq!(
        overlay.message.as_deref(),
        Some("Read the rest first: line 13 of 13 has not been shown.")
    );
    assert!(overlay.rows.iter().any(|row| {
        row.label
            .starts_with("Lines 1–12 of 13 · Ctrl-u/Ctrl-d scroll")
    }));
    assert_eq!(overlay.actions[0].key_hint, "↑/↓/Tab");

    press(&mut app, KeyCode::PageDown);
    present(&mut app, 2);
    let overlay = app.context_overlay().unwrap();
    assert!(overlay.message.is_none());
    assert_eq!(overlay.rows[2].label, "x".repeat(76));
    press(&mut app, KeyCode::Enter);
    assert!(matches!(
        app.context_ui.decision,
        Some(Decision::Proposal { accepted: true, .. })
    ));
}

#[test]
fn buffer_motions_scroll_and_a_jump_does_not_count_as_reading() {
    let mut app = proposal(&"x".repeat(76 * 40), vec![]);
    let top = |app: &App| app.context_ui.surface.as_ref().unwrap().top;
    present(&mut app, 1);
    // 41 lines in a 12-line view: the last view starts at line 30.
    for (keys, expected) in [
        (&[KeyCode::PageDown][..], 12),
        (&[KeyCode::PageDown], 24),
        (&[KeyCode::PageUp], 12),
        (&[KeyCode::Char('g'), KeyCode::Char('g')], 0),
        (&[KeyCode::Char('g'), KeyCode::Char('e')], 29),
        (&[KeyCode::Home], 0),
        (&[KeyCode::End], 29),
        (&[KeyCode::Up], 29),
        (&[KeyCode::Down, KeyCode::Down], 29),
    ] {
        for &code in keys {
            press(&mut app, code);
        }
        assert_eq!(top(&app), expected, "{keys:?}");
    }
    press(&mut app, KeyCode::Up);

    // Lines 1-12 and 30-41 have been shown; the middle has not.
    present(&mut app, 2);
    press(&mut app, KeyCode::Tab);
    press(&mut app, KeyCode::Enter);
    assert!(app.context_ui.decision.is_none());
    assert_eq!(
        app.context_overlay().unwrap().message.as_deref(),
        Some("Read the rest first: line 13 of 41 has not been shown.")
    );
    press(&mut app, KeyCode::PageUp);
    present(&mut app, 3);
    press(&mut app, KeyCode::PageUp);
    present(&mut app, 4);
    assert!(app.context_ui.surface.as_ref().unwrap().fully_reviewed());
    press(&mut app, KeyCode::Enter);
    assert!(matches!(
        app.context_ui.decision,
        Some(Decision::Proposal { accepted: true, .. })
    ));
}

#[test]
fn a_taller_editor_shows_more_of_a_long_proposal() {
    let mut app = proposal(&"x".repeat(76 * 60), vec![]);
    app.note_context_frame(120, 50, 1);
    app.note_context_presented(1);
    let overlay = app.context_overlay().unwrap();
    let text_rows = overlay
        .rows
        .iter()
        .filter(|row| row.label.ends_with(&"x".repeat(92)))
        .count();
    assert_eq!(text_rows, 20, "60% of 50 rows, less fixed controls");
    assert!(
        overlay
            .rows
            .iter()
            .any(|row| row.label.starts_with("Lines 1–20 of 51"))
    );
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
            "word ".repeat(15).trim_end()
        ))],
    );
    let lines = &app.context_ui.surface.as_ref().unwrap().lines;
    assert_eq!(lines[0].value, "ab·cd");
    let prose = lines
        .iter()
        .filter(|line| line.value.starts_with("word") || line.value == "tail")
        .map(|line| line.value.as_str())
        .collect::<Vec<_>>();
    assert_eq!(prose, [&"word ".repeat(15)[..74], "tail"]);
}

#[test]
fn arrows_always_choose_without_scrolling() {
    let mut app = proposal("short", vec![]);
    present(&mut app, 1);
    let rows = app.context_overlay().unwrap().rows.len();
    press(&mut app, KeyCode::Down);
    assert_eq!(app.context_overlay().unwrap().selected, Some(rows - 1));
    press(&mut app, KeyCode::Up);
    assert_eq!(app.context_overlay().unwrap().selected, Some(rows - 2));

    let mut app = proposal(&"x".repeat(76 * 40), vec![]);
    present(&mut app, 1);
    let rows = app.context_overlay().unwrap().rows.len();
    press(&mut app, KeyCode::Down);
    assert_eq!(app.context_ui.surface.as_ref().unwrap().top, 0);
    assert_eq!(app.context_overlay().unwrap().selected, Some(rows - 1));
    press(&mut app, KeyCode::Up);
    assert_eq!(app.context_overlay().unwrap().selected, Some(rows - 2));
    press(&mut app, KeyCode::BackTab);
    assert_eq!(app.context_overlay().unwrap().selected, Some(rows - 1));
    press(&mut app, KeyCode::Escape);
    assert!(matches!(
        app.context_ui.decision,
        Some(Decision::Proposal {
            accepted: false,
            ..
        })
    ));
}

#[test]
fn mcp_permission_shortcuts_highlight_only_the_action_keys() {
    let mut app = App::new(Config::default(), None).unwrap();
    app.context_ui.surface = Some(Surface::new(
        Kind::Grant {
            identity: "agent".into(),
            scopes: BTreeSet::new(),
            remember: false,
        },
        "MCP permissions".into(),
        vec![],
        "",
        app.plugins.attachment_generation,
    ));
    for checked in [false, true] {
        let overlay = app.context_overlay().unwrap();
        let row = overlay
            .rows
            .iter()
            .find(|row| row.label.contains("Remember for this workspace"))
            .unwrap();
        assert!(
            row.label
                .starts_with(if checked { "r [x]" } else { "r [ ]" })
        );
        let highlighted = row
            .label
            .chars()
            .enumerate()
            .filter_map(|(index, character)| row.emphasis.contains(&index).then_some(character))
            .collect::<String>();
        assert_eq!(highlighted, "rxn");
        press(&mut app, KeyCode::Char('r'));
    }
}

#[test]
fn mcp_scope_toggles_keep_prerequisites_and_cycle_identity_without_granting() {
    let mut app = App::new(Config::default(), None).unwrap();
    app.context_ui.identities = vec!["agent".into(), "codex".into()];
    app.context_ui.surface = Some(Surface::new(
        Kind::Grant {
            identity: "agent".into(),
            scopes: BTreeSet::new(),
            remember: false,
        },
        "MCP permissions".into(),
        vec![],
        "",
        app.plugins.attachment_generation,
    ));
    press(&mut app, KeyCode::Char('3'));
    press(&mut app, KeyCode::Char('4'));
    let Kind::Grant { scopes, .. } = &app.context_ui.surface.as_ref().unwrap().kind else {
        panic!()
    };
    assert_eq!(scopes.len(), 4);
    press(&mut app, KeyCode::Char('1'));
    press(&mut app, KeyCode::Char('2'));
    let Kind::Grant { scopes, .. } = &app.context_ui.surface.as_ref().unwrap().kind else {
        panic!()
    };
    assert!(scopes.is_empty());
    press(&mut app, KeyCode::Char('n'));
    assert_eq!(app.context_ui.requested_identity.as_deref(), Some("codex"));
    assert!(app.context_ui.surface.is_none());
    assert!(app.context_ui.decision.is_none());
}

#[test]
fn skipping_review_brings_its_prerequisites_and_leaves_with_them() {
    let mut app = App::new(Config::default(), None).unwrap();
    app.context_ui.surface = Some(Surface::new(
        Kind::Grant {
            identity: "agent".into(),
            scopes: BTreeSet::new(),
            remember: false,
        },
        "MCP permissions".into(),
        vec![],
        "",
        app.plugins.attachment_generation,
    ));
    let scopes = |app: &App| match &app.context_ui.surface.as_ref().unwrap().kind {
        Kind::Grant { scopes, .. } => scopes.clone(),
        Kind::Proposal { .. } | Kind::ChannelBinding(_) => unreachable!(),
    };
    press(&mut app, KeyCode::Char('5'));
    assert_eq!(
        scopes(&app),
        BTreeSet::from([
            Scope::TerminalRead,
            Scope::TerminalPropose,
            Scope::TerminalInsert
        ])
    );
    assert!(
        app.context_overlay()
            .unwrap()
            .rows
            .iter()
            .any(|row| row.label == "5 [x] Skip review: insert proposals directly (no Enter)")
    );
    press(&mut app, KeyCode::Char('4'));
    assert_eq!(scopes(&app), BTreeSet::from([Scope::TerminalRead]));
    press(&mut app, KeyCode::Char('5'));
    press(&mut app, KeyCode::Char('1'));
    assert!(
        scopes(&app).is_empty(),
        "reading is what everything else needs"
    );
    press(&mut app, KeyCode::Char('7'));
    assert_eq!(
        scopes(&app),
        BTreeSet::from([Scope::AgentChannel, Scope::ChannelNudge]),
        "accepting nudges needs the channel"
    );
    assert!(
        app.context_overlay()
            .unwrap()
            .rows
            .iter()
            .any(|row| row.label == "7 [x] Accept channel nudges: a fixed line and Enter")
    );
    press(&mut app, KeyCode::Char('6'));
    assert!(scopes(&app).is_empty());
}

#[test]
fn a_pending_g_is_spent_on_the_next_input_and_otherwise_cancels() {
    let mut app = proposal(&"x".repeat(76 * 40), vec![]);
    let top = |app: &App| app.context_ui.surface.as_ref().unwrap().top;
    present(&mut app, 1);
    press(&mut app, KeyCode::Char('g'));
    press(&mut app, KeyCode::Char('j'));
    assert_eq!(top(&app), 0, "g then j cancels rather than scrolling");
    press(&mut app, KeyCode::Char('g'));
    app.handle_input(InputEvent::Text("pasted".into())).unwrap();
    press(&mut app, KeyCode::Char('e'));
    assert_eq!(top(&app), 0, "a paste spends the pending g");
    press(&mut app, KeyCode::Char('g'));
    press(&mut app, KeyCode::Char('e'));
    assert_eq!(top(&app), 29);
    assert_eq!(
        app.context_ui.surface.as_ref().unwrap().presented_top,
        None,
        "a view not yet presented cannot approve"
    );
}

#[test]
fn a_frame_laid_out_for_an_earlier_area_stops_counting() {
    let mut app = proposal(&"x".repeat(76 * 40), vec![]);
    app.note_context_frame(120, 50, 1);
    app.note_context_frame(80, 22, 2);
    // Frame 1 would show 20 lines, but the window has since shrunk.
    app.note_context_presented(1);
    let surface = app.context_ui.surface.as_ref().unwrap();
    assert!(surface.seen.iter().all(|seen| !seen));
    assert_eq!(surface.presented_top, None);
    app.note_context_presented(2);
    let seen = app.context_ui.surface.as_ref().unwrap().seen.clone();
    assert_eq!(seen.iter().filter(|seen| **seen).count(), 12);
}

#[test]
fn the_session_strip_row_comes_out_of_the_review_not_the_approval_size() {
    let mut app = proposal(&"x".repeat(76 * 40), vec![]);
    app.note_context_frame_in(80, 22, 21, 1);
    app.note_context_presented(1);
    let surface = app.context_ui.surface.as_ref().unwrap();
    assert_eq!(surface.seen.iter().filter(|seen| **seen).count(), 11);
    assert!(
        app.context_overlay().unwrap().message.is_none(),
        "80 x 22 still has room to approve"
    );
}

#[test]
fn page_chords_scroll_without_changing_the_choice() {
    let mut app = proposal(&"x".repeat(76 * 40), vec![]);
    present(&mut app, 1);
    press(&mut app, KeyCode::Down);
    for (key, expected) in [('d', 6), ('f', 18), ('u', 12), ('b', 0)] {
        app.handle_input(InputEvent::Key(KeyStroke::ctrl(key)))
            .unwrap();
        let surface = app.context_ui.surface.as_ref().unwrap();
        assert_eq!(surface.top, expected);
        assert!(surface.accept_selected);
        assert!(app.context_ui.decision.is_none());
    }
    // Plain letters do not borrow less's motions or edit the buffer.
    for key in ['d', 'u', 'j', 'k'] {
        press(&mut app, KeyCode::Char(key));
    }
    assert_eq!(app.context_ui.surface.as_ref().unwrap().top, 0);
}

#[test]
fn wheel_scrolls_only_inside_the_review_body_and_never_approves() {
    use crate::input::{PointerButton, PointerEvent};
    let mut app = proposal(&"x".repeat(76 * 40), vec![]);
    app.note_context_frame(200, 60, 1);
    app.note_context_presented(1);
    press(&mut app, KeyCode::Down);
    let area = app.context_ui.review_area();
    assert_eq!((area.width, area.height), (160, 36));
    let mut wheel = |column, row, kind| {
        app.handle_input(InputEvent::Pointer(PointerEvent {
            column,
            row,
            kind,
            modifiers: Modifiers::NONE,
        }))
        .unwrap();
        let surface = app.context_ui.surface.as_ref().unwrap();
        assert!(surface.accept_selected);
        assert!(app.context_ui.decision.is_none());
        surface.top
    };
    assert_eq!(wheel(0, 0, PointerEventKind::ScrollDown), 0);
    assert_eq!(
        wheel(area.x + 2, area.y + 1, PointerEventKind::ScrollDown),
        0
    );
    assert_eq!(
        wheel(area.x + 2, area.y + 3, PointerEventKind::ScrollDown),
        0
    );
    // This proposal fits at the wider width; use more text for actual scrolling.
    let mut app = proposal(&"x".repeat(76 * 100), vec![]);
    present(&mut app, 1);
    press(&mut app, KeyCode::Down);
    for (kind, top) in [
        (PointerEventKind::ScrollDown, 3),
        (PointerEventKind::ScrollUp, 0),
        (PointerEventKind::Down(PointerButton::Left), 0),
    ] {
        app.handle_input(InputEvent::Pointer(PointerEvent {
            column: 3,
            row: 3,
            kind,
            modifiers: Modifiers::NONE,
        }))
        .unwrap();
        let surface = app.context_ui.surface.as_ref().unwrap();
        assert_eq!(surface.top, top);
        assert!(surface.accept_selected);
        assert!(app.context_ui.decision.is_none());
    }
}

#[test]
fn rewrapping_invalidates_old_read_receipts_and_preserves_every_literal_character() {
    let text = "ab cd\\ef ".repeat(100);
    let mut app = proposal(&text, vec![]);
    present(&mut app, 1);
    app.note_context_frame(200, 60, 2);
    app.note_context_presented(1);
    let surface = app.context_ui.surface.as_ref().unwrap();
    assert_eq!(surface.presented_top, None);
    assert!(surface.seen.iter().all(|seen| !seen));
    let literal = surface
        .lines
        .iter()
        .filter(|line| line.tone == Tone::Literal)
        .map(|line| line.value.as_str())
        .collect::<String>();
    assert_eq!(literal, visible(&text));
    assert!(surface.lines.iter().all(|line| line.value.width() <= 156));
    app.note_context_presented(2);
    assert!(app.context_ui.surface.as_ref().unwrap().fully_reviewed());
}

#[test]
fn review_scroll_uses_the_live_registry_and_its_hint_spellings() {
    use crate::keymap::{Keymap, default_keymap};
    use std::{collections::HashMap, sync::Arc};
    let defaults = default_keymap();
    let old = KeySequence::parse("Ctrl-d").unwrap();
    let new = KeySequence::parse("Ctrl-j").unwrap();
    let mut bindings = defaults.bindings().to_vec();
    let mut spellings = HashMap::new();
    for binding in &mut bindings {
        spellings.insert(binding.sequence.clone(), binding.sequence.clone());
        if let Some(alias) = &binding.alias {
            spellings.insert(alias.clone(), alias.clone());
        }
        if binding.sequence == old && binding.target == BindingTarget::Editor(Command::HalfPageDown)
        {
            binding.sequence = new.clone();
        }
    }
    spellings.insert(old, new);
    let keymap = Keymap::new(bindings).unwrap().with_spelling_metadata(
        KeyStroke::char(' '),
        KeyStroke::ctrl('w'),
        spellings,
    );
    let mut app = proposal(&"x".repeat(76 * 40), vec![]);
    app.set_keymap(Arc::new(keymap));
    present(&mut app, 1);
    app.handle_input(InputEvent::Key(KeyStroke::ctrl('d')))
        .unwrap();
    assert_eq!(app.context_ui.surface.as_ref().unwrap().top, 0);
    app.handle_input(InputEvent::Key(KeyStroke::ctrl('j')))
        .unwrap();
    assert_eq!(app.context_ui.surface.as_ref().unwrap().top, 6);
    assert!(
        app.context_overlay()
            .unwrap()
            .rows
            .iter()
            .any(|row| row.label.contains("Ctrl-u/Ctrl-j scroll"))
    );
}

#[test]
fn a_refreshed_agent_channel_page_shows_its_newest_message() {
    let page = |count: usize| {
        let mut page = String::from("Agent channel\n\nMessages\n\n");
        for n in 1..=count {
            page.push_str(&format!("  12:00:00  codex-1: message {n}\n"));
        }
        page
    };
    let mut app = App::new(Config::default(), None).unwrap();
    app.open_agent_channel_page(&page(3));
    let view = |app: &mut App| {
        app.prepare_view(crate::ui::frame_geometry(ratatui::layout::Rect::new(
            0, 0, 80, 12,
        )))
    };
    view(&mut app);
    assert_eq!(app.active().scroll_row, 0);

    app.refresh_agent_channel_page(&page(40));
    let newest = app.active_buffer().last_row() - 1;
    assert_eq!(app.cursor_position().row, newest);
    assert!(
        app.active_buffer()
            .line_string(newest)
            .contains("message 40")
    );
    view(&mut app);
    let scroll = app.active().scroll_row;
    assert!(scroll > 0 && scroll <= newest && newest < scroll + 12);
}

#[test]
fn terminal_binding_requires_presented_physical_approval() {
    let mut app = ready();
    app.context_ui.surface = Some(Surface::new(
        Kind::ChannelBinding(ChannelBinding {
            participant: "participant".into(),
            connection: 4,
            terminal: Some(99),
        }),
        "Agent terminal binding".into(),
        vec![Detail::note("Confirm terminal #2")],
        "",
        app.plugins.attachment_generation,
    ));
    app.context_ui.surface.as_mut().unwrap().accept_selected = true;
    app.handle_input(InputEvent::Key(KeyStroke::plain(KeyCode::Enter)))
        .unwrap();
    assert!(app.context_ui.decision.is_none(), "unseen review");
    app.note_context_frame(80, 22, 2);
    app.note_context_presented(2);
    app.handle_replayed_input(InputEvent::Key(KeyStroke::plain(KeyCode::Enter)))
        .unwrap();
    app.handle_repeated_input(InputEvent::Key(KeyStroke::plain(KeyCode::Enter)))
        .unwrap();
    app.handle_input(InputEvent::Text("\r\n".into())).unwrap();
    assert!(app.context_ui.decision.is_none());
    app.handle_input(InputEvent::Key(KeyStroke::plain(KeyCode::Enter)))
        .unwrap();
    assert!(matches!(
        app.context_ui.decision,
        Some(Decision::ChannelBinding(ChannelBinding {
            terminal: Some(99),
            ..
        }))
    ));
}

#[test]
fn terminal_binding_command_uses_registry_and_does_not_apply_without_review() {
    let mut app = App::new(Config::default(), None).unwrap();
    for character in ":agent-bind codex-1 2".chars() {
        app.handle_input(InputEvent::Key(KeyStroke::plain(KeyCode::Char(character))))
            .unwrap();
    }
    app.handle_input(InputEvent::Key(KeyStroke::plain(KeyCode::Enter)))
        .unwrap();
    assert_eq!(
        app.context_ui.requested_binding.as_deref(),
        Some("codex-1 2")
    );
    assert!(app.context_ui.decision.is_none());
    app.context_ui.requested_binding = None;
    app.recording_macro = Some('a');
    app.request_channel_binding("codex-1 2".into());
    assert!(app.context_ui.requested_binding.is_none());
}
