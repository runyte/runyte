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

fn encoded(text: &str) -> String {
    text.encode_utf16()
        .map(|unit| format!("\x1b[0;0;{unit};1;0;1_"))
        .collect()
}

#[test]
fn encoded_keys_keep_control_identity_defaults_releases_and_repeats() {
    let mut decoder = Decoder::default();
    let wire = "\x1b[72;35;8;1;8_\x1b[74;36;10;1;8_\x1b[75;37;11;1;8_\x1b[76;38;12;1;8_\x1b[8;14;8;1_\x1b[13;28;13;1_";
    assert_eq!(
        feed(&mut decoder, wire),
        vec![
            key(KeyCode::Char('h'), KeyModifiers::CONTROL),
            key(KeyCode::Char('j'), KeyModifiers::CONTROL),
            key(KeyCode::Char('k'), KeyModifiers::CONTROL),
            key(KeyCode::Char('l'), KeyModifiers::CONTROL),
            key(KeyCode::Backspace, KeyModifiers::NONE),
            key(KeyCode::Enter, KeyModifiers::NONE),
        ]
    );
    assert!(feed(&mut decoder, "\x1b[17;29;;1;8_\x1b[72;35;8;0;8_").is_empty());
    assert_eq!(
        feed(&mut decoder, "\x1b[65;30;97;1;;3_"),
        vec![key(KeyCode::Char('a'), KeyModifiers::NONE); 3]
    );
    assert_eq!(
        feed(&mut decoder, &encoded("😀")),
        vec![key(KeyCode::Char('😀'), KeyModifiers::NONE)]
    );
    assert_eq!(
        feed(&mut decoder, "\x1b[81;16;64;1;9_"),
        vec![key(KeyCode::Char('@'), KeyModifiers::NONE)]
    );
}

#[test]
fn alternate_image_paste_key_survives_native_console_input() {
    let mut decoder = Decoder::default();
    for events in [
        decoder.key(record('v' as u16, 86, true, LEFT_ALT_PRESSED)),
        feed(&mut decoder, "\x1b[86;47;118;1;2;1_"),
    ] {
        assert_eq!(events, vec![key(KeyCode::Char('v'), KeyModifiers::ALT)]);
        assert_eq!(
            convert_event(events[0].clone()).unwrap(),
            Some(InputEvent::Key(crate::input::KeyStroke::alt('v')))
        );
    }
}

#[test]
fn encoded_and_legacy_paste_keep_frame_looking_text_literal() {
    let text = ":quit!\r\n\x08\n\x1b[72;35;8;1;8_😀";
    for wire in [
        format!("\x1b[200~{text}\x1b[201~"),
        encoded(&format!("\x1b[200~{text}\x1b[201~")),
    ] {
        let mut decoder = Decoder::default();
        assert_eq!(feed(&mut decoder, &wire), vec![Event::Paste(text.into())]);
    }
}

#[test]
fn native_frames_are_fragmented_bounded_and_do_not_timeout_into_commands() {
    let mut decoder = Decoder::default();
    assert!(feed(&mut decoder, "\x1b[72;35;").is_empty());
    assert!(
        decoder
            .escape_timeout(Instant::now() + ESCAPE_DELAY)
            .is_none()
    );
    assert_eq!(
        feed(&mut decoder, "8;1;8_"),
        vec![key(KeyCode::Char('h'), KeyModifiers::CONTROL)]
    );
    for frame in [
        "\x1b[65536;0;0;1_",
        "\x1b[72;0;65536;1_",
        "\x1b[72;0;8;2_",
        "\x1b[72;0;8;1;4294967296_",
        "\x1b[72;0;8;1;8;65536_",
        "\x1b[72;0;8;1;8;1;0_",
        "\x1b[-1;0;0;1_",
        "\x1b[+1;0;0;1_",
    ] {
        assert!(feed(&mut decoder, frame).is_empty(), "{frame:?}");
    }
    assert!(feed(&mut decoder, &format!("\x1b[{}_", "1;".repeat(100))).is_empty());
    assert_eq!(
        feed(&mut decoder, "x"),
        vec![key(KeyCode::Char('x'), KeyModifiers::NONE)]
    );
    assert!(feed(&mut decoder, "\x1b").is_empty());
    assert!(
        decoder
            .escape_timeout(Instant::now() + ESCAPE_DELAY)
            .is_none()
    );
    assert_eq!(
        feed(&mut decoder, "[27;1;27;1_"),
        vec![key(KeyCode::Esc, KeyModifiers::NONE)]
    );
}

#[test]
fn explorer_native_chords_honor_fast_panes_and_leave_navigation_keys_intact() {
    use crate::{
        app::{App, FrameGeometry},
        command::parse_colon_command,
        config::Config,
        layout::{Axis, Layout, Rect},
        test_support::TestRuntimeRoot,
    };
    let root = TestRuntimeRoot::new("native-explorer-keys").unwrap();
    let directory = root.join("files");
    std::fs::create_dir(&directory).unwrap();
    std::fs::write(directory.join("note.txt"), "text").unwrap();
    let send = |app: &mut App, vk: u16, unit: u16, state: u32| {
        let wire = format!("\x1b[{vk};0;{unit};1;{state}_");
        for event in feed(&mut Decoder::default(), &wire) {
            app.handle_input(convert_event(event).unwrap().unwrap())
                .unwrap();
        }
    };
    for enabled in [false, true] {
        let mut config = Config::default();
        config.editor.fast_pane_keys = enabled;
        let mut app = App::new_in_project(config, Some(directory.clone()), root.path()).unwrap();
        for command in ["vsplit", "split"] {
            app.execute(parse_colon_command(command).unwrap()).unwrap();
        }
        app.layout = Layout::Split {
            axis: Axis::Horizontal,
            ratio: 32768,
            first: Box::new(Layout::Pane(0)),
            second: Box::new(Layout::Split {
                axis: Axis::Vertical,
                ratio: 32768,
                first: Box::new(Layout::Pane(1)),
                second: Box::new(Layout::Pane(2)),
            }),
        };
        let rect = Rect {
            x: 0,
            y: 0,
            width: 80,
            height: 40,
        };
        app.prepare_view(FrameGeometry {
            screen: rect,
            editor: rect,
            status: Rect::default(),
            message: Rect::default(),
        });
        for (start, vk, unit, end) in [
            (1, 72, 8, 0),
            (1, 74, 10, 2),
            (2, 75, 11, 1),
            (0, 76, 12, 1),
        ] {
            app.active_pane = start;
            send(&mut app, vk, unit, LEFT_CTRL_PRESSED);
            assert_eq!(app.active_pane, if enabled { end } else { start });
            assert_eq!(
                app.active_buffer().directory_root(),
                Some(directory.as_path())
            );
        }
        app.active_pane = 1;
        send(&mut app, 13, 13, 0);
        assert_eq!(
            app.active_buffer().path.as_deref(),
            Some(directory.join("note.txt").as_path())
        );
        app.active_pane = 0;
        send(&mut app, 8, 8, 0);
        assert_eq!(app.active_buffer().directory_root(), Some(root.path()));
    }
    let config_path = root.join("remapped.yaml");
    std::fs::write(&config_path, "keys:\n  rebind:\n    Ctrl-w l: Ctrl-h\n").unwrap();
    let (config, _) = Config::load(Some(&config_path)).unwrap();
    let compiled = crate::keymap::configured::compile(
        config.keys.as_ref().unwrap(),
        &crate::keymap::keymap_for(false),
    );
    assert!(compiled.errors.is_empty(), "{:?}", compiled.errors);
    let mut app = App::new_in_project(config, Some(directory.clone()), root.path()).unwrap();
    app.note_loaded_config(&config_path);
    app.handle_input(InputEvent::Key(crate::keymap::Key::plain(
        crate::input::KeyCode::Escape,
    )))
    .unwrap();
    app.execute(parse_colon_command("vsplit").unwrap()).unwrap();
    app.active_pane = 0;
    let rect = Rect {
        x: 0,
        y: 0,
        width: 80,
        height: 40,
    };
    app.prepare_view(FrameGeometry {
        screen: rect,
        editor: rect,
        status: Rect::default(),
        message: Rect::default(),
    });
    send(&mut app, 72, 8, LEFT_CTRL_PRESSED);
    assert_eq!(app.active_pane, 1, "configured Ctrl+h moves right");
    assert_eq!(
        app.active_buffer().directory_root(),
        Some(directory.as_path())
    );
}

#[test]
fn encoded_paste_survives_timeouts_at_every_wire_boundary() {
    let wire = encoded("\x1b[200~:quit!\r\n\x1b[72;35;8;1;8_\x1b[201~");
    for split in 0..wire.len() {
        let mut decoder = Decoder::for_console();
        let mut events = feed(&mut decoder, &wire[..split]);
        assert!(!decoder.legacy_escape_pending());
        assert!(
            decoder
                .escape_timeout(Instant::now() + ESCAPE_DELAY)
                .is_none(),
            "split {split}"
        );
        events.extend(feed(&mut decoder, &wire[split..]));
        assert_eq!(
            events,
            vec![Event::Paste(":quit!\r\n\x1b[72;35;8;1;8_".into())],
            "split {split}"
        );
    }
}

#[test]
fn raw_paste_cannot_synthesize_its_terminator_from_a_native_frame() {
    let payload = "\x1b[0;0;27;1_[201~:quit!\r";
    let mut decoder = Decoder::for_console();
    assert_eq!(
        feed(&mut decoder, &format!("\x1b[200~{payload}\x1b[201~")),
        vec![Event::Paste("\x1b[0;0;27;1_[201~:quit!\n".into())]
    );
    assert!(!decoder.raw_paste);
    assert_eq!(
        feed(&mut decoder, "\x1b[72;35;8;1;8_"),
        vec![key(KeyCode::Char('h'), KeyModifiers::CONTROL)]
    );
}
