// SPDX-License-Identifier: MPL-2.0

use super::*;
use crate::{input::InputEvent, tui::input::convert_event};

fn record(unit: u16, vk: u16, down: bool, state: u32) -> KEY_EVENT_RECORD {
    KEY_EVENT_RECORD {
        bKeyDown: down.into(),
        wRepeatCount: 1,
        wVirtualKeyCode: vk,
        wVirtualScanCode: 0,
        uChar: KEY_EVENT_RECORD_0 { UnicodeChar: unit },
        dwControlKeyState: state,
    }
}
fn feed(decoder: &mut Decoder, text: &str) -> Vec<Event> {
    text.encode_utf16()
        .flat_map(|unit| decoder.key(record(unit, 0, true, 0)))
        .collect()
}

#[test]
fn native_paste_is_one_literal_event_across_record_boundaries() {
    let mut decoder = Decoder::default();
    assert!(feed(&mut decoder, "\x1b[20").is_empty());
    assert!(feed(&mut decoder, "0~:quit!\r\n\t\x03café ").is_empty());
    // ConPTY reports non-BMP text through two Alt-code key releases.
    for unit in "😀".encode_utf16() {
        assert!(
            decoder
                .key(record(0, 0x12, true, LEFT_ALT_PRESSED))
                .is_empty()
        );
        assert!(decoder.key(record(unit, 0x12, false, 0)).is_empty());
    }
    assert!(
        decoder
            .escape_timeout(Instant::now() + Duration::from_secs(1))
            .is_none()
    );
    let events = feed(&mut decoder, "\x1b[201~");
    assert_eq!(events.len(), 1);
    assert_eq!(
        convert_event(events[0].clone()).unwrap(),
        Some(InputEvent::Text(":quit!\r\n\t\x03café 😀".into()))
    );
    assert_eq!(
        feed(&mut decoder, "x"),
        vec![key(KeyCode::Char('x'), KeyModifiers::NONE)]
    );
}

#[test]
fn conpty_enter_records_become_lines_without_doubling_crlf() {
    let mut decoder = Decoder::default();
    assert_eq!(
        feed(&mut decoder, "\x1b[200~one\rtwo\r\nthree\nfour\x1b[201~"),
        vec![Event::Paste("one\ntwo\r\nthree\nfour".into())]
    );
}

#[test]
fn oversize_paste_is_bounded_and_drained_before_next_key() {
    let mut decoder = Decoder::default();
    feed(&mut decoder, "\x1b[200~");
    assert!(feed(&mut decoder, &"x".repeat(MAX_PASTE_UNITS + 100)).is_empty());
    assert_eq!(decoder.paste.as_ref().unwrap().len(), MAX_PASTE_UNITS);
    let events = feed(&mut decoder, "\x1b[201~");
    assert!(
        matches!(&events[..], [Event::Paste(text)] if text.len() > crate::input::MAX_TEXT_INPUT_BYTES)
    );
    assert!(decoder.paste.is_none());
}

#[test]
fn unicode_altgr_repeats_and_key_releases() {
    let mut decoder = Decoder::default();
    assert_eq!(
        decoder.key(record(
            '@' as u16,
            81,
            true,
            RIGHT_ALT_PRESSED | LEFT_CTRL_PRESSED
        )),
        vec![key(KeyCode::Char('@'), KeyModifiers::NONE)]
    );
    assert!(decoder.key(record('@' as u16, 81, false, 0)).is_empty());
    assert_eq!(
        decoder.key(record('é' as u16, 0x12, false, 0)),
        vec![key(KeyCode::Char('é'), KeyModifiers::NONE)]
    );
    let mut repeated = record('j' as u16, 74, true, 0);
    repeated.wRepeatCount = 3;
    assert_eq!(
        decoder.key(repeated),
        vec![key(KeyCode::Char('j'), KeyModifiers::NONE); 3]
    );
    assert_eq!(
        feed(&mut decoder, "😀"),
        vec![key(KeyCode::Char('😀'), KeyModifiers::NONE)]
    );
}

#[test]
fn alt_numpad_dead_keys_and_modifiers_never_insert_nul() {
    let mut decoder = Decoder::default();
    for vk in [0x12, 0x66, 0x65] {
        assert!(
            decoder
                .key(record(0, vk, true, LEFT_ALT_PRESSED))
                .is_empty()
        );
    }
    assert_eq!(
        decoder.key(record(65, 0x12, false, 0)),
        vec![key(KeyCode::Char('A'), KeyModifiers::NONE)]
    );
    for vk in [0xde, 65, 0x5b, 0x5c] {
        assert!(decoder.key(record(0, vk, true, 0)).is_empty());
    }
    assert_eq!(
        decoder.key(record(0, 32, true, LEFT_CTRL_PRESSED)),
        vec![key(KeyCode::Char(' '), KeyModifiers::CONTROL)]
    );
    assert_eq!(
        feed(&mut decoder, "\0"),
        vec![key(KeyCode::Char(' '), KeyModifiers::CONTROL)]
    );
    feed(&mut decoder, "\x1b[200~");
    assert!(decoder.key(record(0, 0x5b, true, 0)).is_empty());
    assert_eq!(
        feed(&mut decoder, "\x1b[201~"),
        vec![Event::Paste(String::new())]
    );
}

#[test]
fn sgr_mouse_coordinates_modifiers_drag_release_and_wheel() {
    let mut decoder = Decoder::default();
    let events = feed(
        &mut decoder,
        "\x1b[<4;12;8M\x1b[<32;13;9M\x1b[<0;13;9m\x1b[<65;1;1M",
    );
    assert_eq!(
        events,
        vec![
            Event::Mouse(MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                modifiers: KeyModifiers::SHIFT,
                column: 11,
                row: 7
            }),
            Event::Mouse(MouseEvent {
                kind: MouseEventKind::Drag(MouseButton::Left),
                modifiers: KeyModifiers::NONE,
                column: 12,
                row: 8
            }),
            Event::Mouse(MouseEvent {
                kind: MouseEventKind::Up(MouseButton::Left),
                modifiers: KeyModifiers::NONE,
                column: 12,
                row: 8
            }),
            Event::Mouse(MouseEvent {
                kind: MouseEventKind::ScrollDown,
                modifiers: KeyModifiers::NONE,
                column: 0,
                row: 0
            }),
        ]
    );
}

#[test]
fn vt_navigation_controls_focus_and_escape() {
    let mut decoder = Decoder::default();
    assert_eq!(
        feed(&mut decoder, "\x1b[1;5D\x1b[3~\x03\x1bOP\x1b[Z\x1b[I"),
        vec![
            key(KeyCode::Left, KeyModifiers::CONTROL),
            key(KeyCode::Delete, KeyModifiers::NONE),
            key(KeyCode::Char('c'), KeyModifiers::CONTROL),
            key(KeyCode::F(1), KeyModifiers::NONE),
            key(KeyCode::BackTab, KeyModifiers::SHIFT),
            Event::FocusGained
        ]
    );
    assert!(feed(&mut decoder, "\x1b").is_empty());
    assert_eq!(
        decoder.escape_timeout(Instant::now() + ESCAPE_DELAY),
        Some(key(KeyCode::Esc, KeyModifiers::NONE))
    );
    assert!(feed(&mut decoder, "\x1b[999~").is_empty());
}
