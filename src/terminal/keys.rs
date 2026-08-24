// SPDX-License-Identifier: MPL-2.0

//! Turning an editor keystroke back into the bytes a tty child expects.
//!
//! Runyte asks the outer terminal for the kitty keyboard protocol and receives
//! keys already decoded into [`KeyStroke`]s. A child on a pty has asked for no
//! such thing, so what it gets here is the ordinary xterm encoding, in the
//! variant its own modes have selected. The round trip is lossy in exactly the
//! places the outer protocol was meant to fix — `Ctrl-i` and `Tab` are one
//! byte, as are `Ctrl-m` and `Enter` — and that is a property of the pty, not
//! something this module can recover.

use crate::input::{KeyCode, KeyStroke, Modifiers};

use super::emulator::Modes;

/// Encodes one keystroke, or reports that it carries nothing to send.
pub fn encode(key: KeyStroke, modes: Modes) -> Option<Vec<u8>> {
    let modifiers = key.modifiers;
    let alt = modifiers.contains(Modifiers::ALT);
    let control = modifiers.contains(Modifiers::CONTROL);
    let shift = modifiers.contains(Modifiers::SHIFT);
    let mut bytes = match key.code {
        KeyCode::Char(character) => {
            if control {
                control_byte(character).map(|byte| vec![byte])?
            } else {
                let mut buffer = [0_u8; 4];
                character.encode_utf8(&mut buffer).as_bytes().to_vec()
            }
        }
        KeyCode::Enter => vec![b'\r'],
        KeyCode::Tab => vec![b'\t'],
        KeyCode::BackTab => return Some(b"\x1b[Z".to_vec()),
        // The pty's erase character. Terminals disagree about `Backspace`, and
        // `DEL` is what the overwhelming majority of shells and readline
        // configurations are set up for.
        KeyCode::Backspace => vec![0x7f],
        KeyCode::Escape => vec![0x1b],
        KeyCode::Null => vec![0],
        KeyCode::Up => return Some(cursor_key(b'A', modes, modifiers)),
        KeyCode::Down => return Some(cursor_key(b'B', modes, modifiers)),
        KeyCode::Right => return Some(cursor_key(b'C', modes, modifiers)),
        KeyCode::Left => return Some(cursor_key(b'D', modes, modifiers)),
        KeyCode::Home => return Some(cursor_key(b'H', modes, modifiers)),
        KeyCode::End => return Some(cursor_key(b'F', modes, modifiers)),
        KeyCode::Insert => return Some(tilde_key(2, modifiers)),
        KeyCode::Delete => return Some(tilde_key(3, modifiers)),
        KeyCode::PageUp => return Some(tilde_key(5, modifiers)),
        KeyCode::PageDown => return Some(tilde_key(6, modifiers)),
        KeyCode::Function(number) => return function_key(number, modifiers),
        // Modifier releases, media keys, and lock keys have no tty encoding.
        _ => return None,
    };
    if alt {
        bytes.insert(0, 0x1b);
    }
    let _ = shift;
    Some(bytes)
}

/// Wraps pasted text so a program reading it treats it as data.
///
/// Without the brackets a multi-line paste into a shell runs every line but
/// the last, which is the difference between pasting a script and executing
/// one. A child that has not asked for bracketed paste gets the text bare,
/// because the brackets would then be printed.
///
/// The two cases differ in more than the brackets. Bare text is going to a
/// line discipline, which is what a person's keyboard talks to, and there a
/// line ending has to be the carriage return that Enter produces or the shell
/// never sees a line at all. Between the brackets the payload is data: a
/// program reading raw input can tell `\r` from `\n`, and a TUI editor handed
/// carriage returns where the selection had line breaks would insert the wrong
/// thing. So the brackets carry the text exactly as it was selected.
pub fn encode_paste(text: &str, modes: Modes) -> Vec<u8> {
    if modes.bracketed_paste {
        let mut bytes = b"\x1b[200~".to_vec();
        bytes.extend_from_slice(text.as_bytes());
        bytes.extend_from_slice(b"\x1b[201~");
        return bytes;
    }
    text.replace("\r\n", "\r").replace('\n', "\r").into_bytes()
}

/// The xterm modifier parameter: one plus a bit per held modifier.
fn modifier_parameter(modifiers: Modifiers) -> u8 {
    let mut value = 1;
    if modifiers.contains(Modifiers::SHIFT) {
        value += 1;
    }
    if modifiers.contains(Modifiers::ALT) {
        value += 2;
    }
    if modifiers.contains(Modifiers::CONTROL) {
        value += 4;
    }
    value
}

fn cursor_key(final_byte: u8, modes: Modes, modifiers: Modifiers) -> Vec<u8> {
    let parameter = modifier_parameter(modifiers);
    if parameter > 1 {
        // A modified cursor key is always the CSI form, even in application
        // mode: `SS3` has nowhere to put the parameter.
        return format!("\x1b[1;{parameter}{}", final_byte as char).into_bytes();
    }
    if modes.application_cursor_keys {
        vec![0x1b, b'O', final_byte]
    } else {
        vec![0x1b, b'[', final_byte]
    }
}

fn tilde_key(number: u8, modifiers: Modifiers) -> Vec<u8> {
    let parameter = modifier_parameter(modifiers);
    if parameter > 1 {
        format!("\x1b[{number};{parameter}~").into_bytes()
    } else {
        format!("\x1b[{number}~").into_bytes()
    }
}

fn function_key(number: u8, modifiers: Modifiers) -> Option<Vec<u8>> {
    let parameter = modifier_parameter(modifiers);
    let final_byte = match number {
        1 => b'P',
        2 => b'Q',
        3 => b'R',
        4 => b'S',
        _ => {
            let code = match number {
                5 => 15,
                6 => 17,
                7 => 18,
                8 => 19,
                9 => 20,
                10 => 21,
                11 => 23,
                12 => 24,
                _ => return None,
            };
            return Some(tilde_key(code, modifiers));
        }
    };
    if parameter > 1 {
        Some(format!("\x1b[1;{parameter}{}", final_byte as char).into_bytes())
    } else {
        Some(vec![0x1b, b'O', final_byte])
    }
}

/// The control byte a `Ctrl`-modified character produces on a tty.
fn control_byte(character: char) -> Option<u8> {
    let byte = match character {
        ' ' | '@' => 0,
        '[' => 27,
        '\\' => 28,
        ']' => 29,
        '^' => 30,
        '_' | '/' => 31,
        '?' => 127,
        'a'..='z' => character as u8 - b'a' + 1,
        'A'..='Z' => character as u8 - b'A' + 1,
        // `Ctrl` with a digit or a symbol has no control byte. Sending the
        // character bare would be worse than sending nothing, because a shell
        // would insert it.
        _ => return None,
    };
    Some(byte)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn modes() -> Modes {
        Modes::default()
    }

    #[test]
    fn plain_characters_are_their_own_utf8() {
        assert_eq!(encode(KeyStroke::char('a'), modes()), Some(b"a".to_vec()));
        assert_eq!(
            encode(KeyStroke::char('漢'), modes()),
            Some("漢".as_bytes().to_vec())
        );
    }

    #[test]
    fn control_characters_use_the_tty_control_byte() {
        assert_eq!(encode(KeyStroke::ctrl('c'), modes()), Some(vec![3]));
        assert_eq!(encode(KeyStroke::ctrl('C'), modes()), Some(vec![3]));
        assert_eq!(encode(KeyStroke::ctrl(' '), modes()), Some(vec![0]));
        assert_eq!(encode(KeyStroke::ctrl('1'), modes()), None);
    }

    #[test]
    fn alt_prefixes_an_escape() {
        assert_eq!(encode(KeyStroke::alt('b'), modes()), Some(vec![0x1b, b'b']));
    }

    #[test]
    fn cursor_keys_follow_the_childs_application_mode() {
        let key = KeyStroke::plain(KeyCode::Up);
        assert_eq!(encode(key, modes()), Some(b"\x1b[A".to_vec()));
        let mut application = modes();
        application.application_cursor_keys = true;
        assert_eq!(encode(key, application), Some(b"\x1bOA".to_vec()));
    }

    #[test]
    fn a_modified_cursor_key_is_always_the_csi_form() {
        let mut application = modes();
        application.application_cursor_keys = true;
        let key = KeyStroke::new(KeyCode::Right, Modifiers::CONTROL);
        assert_eq!(encode(key, application), Some(b"\x1b[1;5C".to_vec()));
    }

    #[test]
    fn editing_keys_use_their_tilde_codes() {
        assert_eq!(
            encode(KeyStroke::plain(KeyCode::Delete), modes()),
            Some(b"\x1b[3~".to_vec())
        );
        assert_eq!(
            encode(KeyStroke::new(KeyCode::PageUp, Modifiers::SHIFT), modes()),
            Some(b"\x1b[5;2~".to_vec())
        );
    }

    #[test]
    fn function_keys_split_between_ss3_and_tilde_forms() {
        assert_eq!(
            encode(KeyStroke::plain(KeyCode::Function(1)), modes()),
            Some(b"\x1bOP".to_vec())
        );
        assert_eq!(
            encode(KeyStroke::plain(KeyCode::Function(5)), modes()),
            Some(b"\x1b[15~".to_vec())
        );
    }

    #[test]
    fn backspace_sends_delete_and_enter_sends_a_carriage_return() {
        assert_eq!(
            encode(KeyStroke::plain(KeyCode::Backspace), modes()),
            Some(vec![0x7f])
        );
        assert_eq!(
            encode(KeyStroke::plain(KeyCode::Enter), modes()),
            Some(vec![b'\r'])
        );
    }

    #[test]
    fn paste_is_bracketed_only_when_the_child_asked_for_it() {
        let mut bracketed = modes();
        bracketed.bracketed_paste = true;
        assert_eq!(
            encode_paste("one\ntwo", bracketed),
            b"\x1b[200~one\ntwo\x1b[201~".to_vec()
        );
        assert_eq!(encode_paste("one\ntwo", modes()), b"one\rtwo".to_vec());
    }

    /// Between the brackets the payload is data, and a program reading raw
    /// input can tell the two bytes apart. Outside them it is keystrokes, and
    /// a line discipline only recognises a carriage return as Enter.
    #[test]
    fn bracketed_paste_keeps_the_line_endings_it_was_given() {
        let mut bracketed = modes();
        bracketed.bracketed_paste = true;
        assert_eq!(
            encode_paste("one\r\ntwo\n", bracketed),
            b"\x1b[200~one\r\ntwo\n\x1b[201~".to_vec()
        );
        assert_eq!(
            encode_paste("one\r\ntwo\n", modes()),
            b"one\rtwo\r".to_vec()
        );
    }
}
