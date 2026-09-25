// SPDX-License-Identifier: MPL-2.0

use std::sync::{Arc, Mutex};

use crossterm::event::Event;
use runyte::{
    app::App, clipboard::SystemClipboard, command::Mode, config::Config, input::KeyStroke,
    test_support::TestRuntimeRoot, tui::input::convert_event,
};

struct Clipboard(Arc<Mutex<String>>);

impl SystemClipboard for Clipboard {
    fn read(&mut self) -> anyhow::Result<String> {
        Ok(self.0.lock().unwrap().clone())
    }

    fn write(&mut self, text: &str) -> anyhow::Result<()> {
        *self.0.lock().unwrap() = text.to_owned();
        Ok(())
    }
}

#[test]
fn explorer_clipboard_round_trip_through_terminal_paste_preserves_rows() {
    let root = TestRuntimeRoot::new("explorer-paste").unwrap();
    for name in ["alpha.txt", "βeta.txt"] {
        std::fs::write(root.path().join(name), "").unwrap();
    }
    let clipboard = Arc::new(Mutex::new(String::new()));
    let mut explorer = App::new_in_project(
        Config::default(),
        Some(root.path().to_path_buf()),
        root.path(),
    )
    .unwrap();
    explorer.set_system_clipboard(Box::new(Clipboard(clipboard.clone())));
    for key in ['x', 'x', ' ', 'c', 'y'] {
        explorer.handle_key(KeyStroke::char(key)).unwrap();
    }
    let copied = clipboard.lock().unwrap().clone();
    assert_eq!(copied, "alpha.txt\nβeta.txt");

    let mut document = App::new_in_project(Config::default(), None, root.path()).unwrap();
    document.handle_key(KeyStroke::char('i')).unwrap();
    // macOS terminal paste actions can encode clipboard LF as Enter/CR.
    let event = convert_event(Event::Paste(copied.replace('\n', "\r")))
        .unwrap()
        .unwrap();
    document.handle_input(event).unwrap();
    assert_eq!(document.active_buffer().text().to_string(), copied);
    assert_eq!(document.mode, Mode::Insert);
}

#[test]
fn terminal_paste_preserves_lf_crlf_and_literal_commands() {
    for (wire, expected) in [
        ("one\rtwo\r", "one\ntwo\n"),
        ("one\r\ntwo\n", "one\r\ntwo\n"),
        ("\r\r\n\n:quit!\tλ", "\n\r\n\n:quit!\tλ"),
    ] {
        assert_eq!(
            convert_event(Event::Paste(wire.to_owned())).unwrap(),
            Some(runyte::input::InputEvent::Text(expected.to_owned())),
        );
    }
}
