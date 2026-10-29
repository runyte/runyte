// SPDX-License-Identifier: MPL-2.0

use super::*;
use crate::{config::Config, input::KeyStroke};

/// A grant review for `agent` with `details` below its toggles.
fn grant(details: Vec<Detail>) -> App {
    let mut app = App::new(Config::default(), None).unwrap();
    app.context_ui.surface = Some(Surface::new(
        Grant {
            identity: "agent".into(),
            permissions: BTreeSet::new(),
            remember: false,
        },
        "MCP permissions".into(),
        details,
        app.plugins.attachment_generation,
    ));
    app
}

/// A grant review whose body is `lines` full rows long.
fn long_grant(lines: usize) -> App {
    grant(vec![Detail::note("x".repeat(64 * lines))])
}

/// A short review, shown, with Apply selected.
fn ready() -> App {
    let mut app = grant(vec![]);
    present(&mut app, 1);
    app.context_ui.surface.as_mut().unwrap().accept_selected = true;
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

fn permissions(app: &App) -> BTreeSet<Permission> {
    app.context_ui
        .surface
        .as_ref()
        .unwrap()
        .grant
        .permissions
        .clone()
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
fn repeated_enter_cannot_approve_but_repeat_navigation_remains_live() {
    let mut app = grant(vec![]);
    present(&mut app, 1);
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
        Some(Decision::Grant { .. })
    ));

    let mut long = long_grant(41);
    present(&mut long, 1);
    long.handle_repeated_input(InputEvent::Key(KeyStroke::ctrl('d')))
        .unwrap();
    assert_eq!(long.context_ui.surface.as_ref().unwrap().top, 5);
    long.handle_repeated_input(InputEvent::Key(KeyStroke::ctrl('u')))
        .unwrap();
    assert_eq!(long.context_ui.surface.as_ref().unwrap().top, 0);
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
    assert_eq!(
        app.context_overlay().unwrap().message.as_deref(),
        Some("Enlarge terminal to at least 80 × 24 to approve")
    );
    app.plugins.attachment_generation += 1;
    app.handle_input(InputEvent::Key(KeyStroke::plain(KeyCode::Enter)))
        .unwrap();
    assert!(app.context_ui.decision.is_none());
    assert!(!app.context_overlay_active());
    assert_eq!(visible(" \\n界\u{202e}"), "·\\\\n\\u{754c}\\u{202e}");
    assert_ne!(visible("\\u{202e}"), visible("\u{202e}"));
}

#[test]
fn an_applied_grant_carries_the_chosen_permissions_and_remember() {
    let mut app = grant(vec![]);
    present(&mut app, 1);
    let rows = app.context_overlay().unwrap().rows.len();
    assert_eq!(
        app.context_overlay().unwrap().selected,
        Some(rows - 2),
        "Reject starts selected"
    );
    press(&mut app, KeyCode::Char('2'));
    press(&mut app, KeyCode::Char('r'));
    press(&mut app, KeyCode::Tab);
    press(&mut app, KeyCode::Enter);
    let Some(Decision::Grant {
        identity,
        permissions,
        remember,
    }) = &app.context_ui.decision
    else {
        panic!("no grant decided");
    };
    assert_eq!(identity, "agent");
    assert_eq!(
        *permissions,
        BTreeSet::from([Permission::Read, Permission::Write])
    );
    assert!(*remember);
    assert!(!app.context_overlay_active());

    // Reject decides nothing, and neither does Escape; x revokes.
    let mut app = grant(vec![]);
    present(&mut app, 1);
    press(&mut app, KeyCode::Enter);
    assert!(app.context_ui.decision.is_none());
    assert!(!app.context_overlay_active());
    let mut app = grant(vec![]);
    press(&mut app, KeyCode::Escape);
    assert!(app.context_ui.decision.is_none());
    assert!(!app.context_overlay_active());
    let mut app = grant(vec![]);
    press(&mut app, KeyCode::Char('x'));
    assert!(matches!(
        &app.context_ui.decision,
        Some(Decision::Revoke(identity)) if identity == "agent"
    ));
}

#[test]
fn refused_apply_names_the_unread_line_until_every_line_is_shown() {
    // One line more than the smallest approving area shows.
    let mut app = long_grant(12);
    present(&mut app, 1);
    press(&mut app, KeyCode::Tab);
    press(&mut app, KeyCode::Enter);
    assert!(app.context_ui.decision.is_none());
    let overlay = app.context_overlay().unwrap();
    assert_eq!(
        overlay.message.as_deref(),
        Some("Read the rest first: line 12 of 12 has not been shown.")
    );
    assert!(overlay.rows.iter().any(|row| {
        row.label
            .starts_with("Lines 1–11 of 12 · Ctrl-u/Ctrl-d scroll")
    }));
    assert_eq!(overlay.actions[0].key_hint, "↑/↓/Tab");

    press(&mut app, KeyCode::PageDown);
    present(&mut app, 2);
    let overlay = app.context_overlay().unwrap();
    assert!(overlay.message.is_none());
    press(&mut app, KeyCode::Enter);
    assert!(matches!(
        app.context_ui.decision,
        Some(Decision::Grant { .. })
    ));
}

#[test]
fn buffer_motions_scroll_and_a_jump_does_not_count_as_reading() {
    let mut app = long_grant(41);
    let top = |app: &App| app.context_ui.surface.as_ref().unwrap().top;
    present(&mut app, 1);
    // 41 lines in an 11-line view: the last view starts at line 31.
    for (keys, expected) in [
        (&[KeyCode::PageDown][..], 11),
        (&[KeyCode::PageDown], 22),
        (&[KeyCode::PageUp], 11),
        (&[KeyCode::Char('g'), KeyCode::Char('g')], 0),
        (&[KeyCode::Char('g'), KeyCode::Char('e')], 30),
        (&[KeyCode::Home], 0),
        (&[KeyCode::End], 30),
        (&[KeyCode::Up], 30),
        (&[KeyCode::Down, KeyCode::Down], 30),
    ] {
        for &code in keys {
            press(&mut app, code);
        }
        assert_eq!(top(&app), expected, "{keys:?}");
    }
    press(&mut app, KeyCode::Up);

    // Lines 1-11 and 31-41 have been shown; the middle has not.
    present(&mut app, 2);
    press(&mut app, KeyCode::Tab);
    press(&mut app, KeyCode::Enter);
    assert!(app.context_ui.decision.is_none());
    assert_eq!(
        app.context_overlay().unwrap().message.as_deref(),
        Some("Read the rest first: line 12 of 41 has not been shown.")
    );
    press(&mut app, KeyCode::PageUp);
    present(&mut app, 3);
    press(&mut app, KeyCode::PageUp);
    present(&mut app, 4);
    assert!(app.context_ui.surface.as_ref().unwrap().fully_reviewed());
    press(&mut app, KeyCode::Enter);
    assert!(matches!(
        app.context_ui.decision,
        Some(Decision::Grant { .. })
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
fn prose_wraps_between_words_and_long_words_anywhere() {
    let app = grant(vec![
        Detail::value("Identity", "agent".into()),
        Detail::note(format!("{} tail", "word ".repeat(15).trim_end())),
        Detail::note("z".repeat(70)),
    ]);
    let lines = &app.context_ui.surface.as_ref().unwrap().lines;
    assert_eq!(lines[0].label, "Identity");
    assert_eq!(lines[0].value, "agent");
    let prose = lines
        .iter()
        .filter(|line| line.value.starts_with("word") || line.value == "tail")
        .map(|line| line.value.as_str())
        .collect::<Vec<_>>();
    assert_eq!(prose, [&"word ".repeat(13)[..64], "word word tail"]);
    let long = lines
        .iter()
        .filter(|line| line.value.starts_with('z'))
        .map(|line| line.value.len())
        .collect::<Vec<_>>();
    assert_eq!(long, [64, 6]);
}

#[test]
fn arrows_always_choose_without_scrolling() {
    let mut app = grant(vec![]);
    present(&mut app, 1);
    let rows = app.context_overlay().unwrap().rows.len();
    press(&mut app, KeyCode::Down);
    assert_eq!(app.context_overlay().unwrap().selected, Some(rows - 1));
    press(&mut app, KeyCode::Up);
    assert_eq!(app.context_overlay().unwrap().selected, Some(rows - 2));

    let mut app = long_grant(41);
    present(&mut app, 1);
    let rows = app.context_overlay().unwrap().rows.len();
    press(&mut app, KeyCode::Down);
    assert_eq!(app.context_ui.surface.as_ref().unwrap().top, 0);
    assert_eq!(app.context_overlay().unwrap().selected, Some(rows - 1));
    press(&mut app, KeyCode::Up);
    assert_eq!(app.context_overlay().unwrap().selected, Some(rows - 2));
    press(&mut app, KeyCode::BackTab);
    assert_eq!(app.context_overlay().unwrap().selected, Some(rows - 1));
}

#[test]
fn mcp_permission_shortcuts_highlight_only_the_action_keys() {
    let mut app = grant(vec![]);
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
fn write_brings_read_and_leaves_with_it_and_n_cycles_identity_without_granting() {
    let mut app = grant(vec![]);
    app.context_ui.identities = vec!["agent".into(), "codex".into()];
    let toggles = |app: &App| {
        app.context_overlay().unwrap().rows[..2]
            .iter()
            .map(|row| row.label.clone())
            .collect::<Vec<_>>()
    };
    assert_eq!(
        toggles(&app),
        [
            "1 [ ] Read: buffers, terminals and history, the agent channel",
            "2 [ ] Write: edit buffers, type in terminals, press Enter",
        ],
        "nothing is preselected"
    );
    press(&mut app, KeyCode::Char('2'));
    assert_eq!(
        permissions(&app),
        BTreeSet::from([Permission::Read, Permission::Write])
    );
    assert!(toggles(&app)[1].starts_with("2 [x]"));
    press(&mut app, KeyCode::Char('2'));
    assert_eq!(permissions(&app), BTreeSet::from([Permission::Read]));
    press(&mut app, KeyCode::Char('2'));
    press(&mut app, KeyCode::Char('1'));
    assert!(permissions(&app).is_empty(), "writing needs reading");
    for key in ['3', '7'] {
        press(&mut app, KeyCode::Char(key));
        assert!(permissions(&app).is_empty(), "{key} is no permission");
    }
    press(&mut app, KeyCode::Char('n'));
    assert_eq!(app.context_ui.requested_identity.as_deref(), Some("codex"));
    assert!(app.context_ui.surface.is_none());
    assert!(app.context_ui.decision.is_none());
}

#[test]
fn a_pending_g_is_spent_on_the_next_input_and_otherwise_cancels() {
    let mut app = long_grant(41);
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
    assert_eq!(top(&app), 30);
    assert_eq!(
        app.context_ui.surface.as_ref().unwrap().presented_top,
        None,
        "a view not yet presented cannot approve"
    );
}

#[test]
fn a_frame_laid_out_for_an_earlier_area_stops_counting() {
    let mut app = long_grant(41);
    app.note_context_frame(120, 50, 1);
    app.note_context_frame(80, 22, 2);
    // Frame 1 would show more lines, but the window has since shrunk.
    app.note_context_presented(1);
    let surface = app.context_ui.surface.as_ref().unwrap();
    assert!(surface.seen.iter().all(|seen| !seen));
    assert_eq!(surface.presented_top, None);
    app.note_context_presented(2);
    let seen = app.context_ui.surface.as_ref().unwrap().seen.clone();
    assert_eq!(seen.iter().filter(|seen| **seen).count(), 11);
}

#[test]
fn the_session_strip_row_comes_out_of_the_review_not_the_approval_size() {
    let mut app = long_grant(41);
    app.note_context_frame_in(80, 22, 21, 1);
    app.note_context_presented(1);
    let surface = app.context_ui.surface.as_ref().unwrap();
    assert_eq!(surface.seen.iter().filter(|seen| **seen).count(), 10);
    assert!(
        app.context_overlay().unwrap().message.is_none(),
        "80 x 22 still has room to approve"
    );
}

#[test]
fn page_chords_scroll_without_changing_the_choice() {
    let mut app = long_grant(41);
    present(&mut app, 1);
    press(&mut app, KeyCode::Down);
    for (key, expected) in [('d', 5), ('f', 16), ('u', 11), ('b', 0)] {
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
fn pointer_input_never_scrolls_or_approves() {
    use crate::input::{PointerButton, PointerEvent, PointerEventKind};
    let mut app = long_grant(41);
    present(&mut app, 1);
    press(&mut app, KeyCode::Down);
    for kind in [
        PointerEventKind::ScrollDown,
        PointerEventKind::Down(PointerButton::Left),
    ] {
        app.handle_input(InputEvent::Pointer(PointerEvent {
            column: 10,
            row: 5,
            kind,
            modifiers: Modifiers::NONE,
        }))
        .unwrap();
        let surface = app.context_ui.surface.as_ref().unwrap();
        assert_eq!(surface.top, 0);
        assert!(surface.accept_selected);
        assert!(app.context_ui.decision.is_none());
    }
}

#[test]
fn agent_bind_is_no_longer_a_command() {
    let mut app = App::new(Config::default(), None).unwrap();
    for character in ":agent-bind codex-1 2".chars() {
        app.handle_input(InputEvent::Key(KeyStroke::plain(KeyCode::Char(character))))
            .unwrap();
    }
    app.handle_input(InputEvent::Key(KeyStroke::plain(KeyCode::Enter)))
        .unwrap();
    assert!(app.context_ui.surface.is_none());
    assert!(app.context_ui.decision.is_none());
    assert!(app.context_ui.requested_identity.is_none());
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
    let mut app = long_grant(41);
    app.set_keymap(Arc::new(keymap));
    present(&mut app, 1);
    app.handle_input(InputEvent::Key(KeyStroke::ctrl('d')))
        .unwrap();
    assert_eq!(app.context_ui.surface.as_ref().unwrap().top, 0);
    app.handle_input(InputEvent::Key(KeyStroke::ctrl('j')))
        .unwrap();
    assert_eq!(app.context_ui.surface.as_ref().unwrap().top, 5);
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
