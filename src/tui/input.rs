// SPDX-License-Identifier: MPL-2.0

//! Conversion from Crossterm events to Runyte-owned input values.
//!
//! This is the only place that should need to understand both vocabularies.
//! Focus, resize, key-release, and standalone modifier events are not editing
//! input and therefore convert to `None`. Other presses and held-key repeats
//! become the same owned key; the TUI loop retains their lifecycle distinction
//! only long enough to pace repeated cursor motions. Mouse events become owned
//! pointer values.
//! Bracketed paste maps to one literal
//! [`InputEvent::Text`] value rather than becoming a sequence of bindable
//! character keys.

use std::{error::Error, fmt};

use crossterm::event::{
    Event as CrosstermEvent, KeyCode as CrosstermKeyCode, KeyEvent as CrosstermKeyEvent,
    KeyEventKind as CrosstermKeyEventKind, KeyModifiers as CrosstermModifiers,
    MediaKeyCode as CrosstermMediaKey, ModifierKeyCode as CrosstermModifierKey,
    MouseButton as CrosstermMouseButton, MouseEvent as CrosstermMouseEvent,
    MouseEventKind as CrosstermMouseEventKind,
};

use crate::input::{
    InputEvent, KeyCode, KeyStroke, MediaKey, ModifierKey, Modifiers, PointerButton, PointerEvent,
    PointerEventKind,
};

/// A terminal key carried modifier bits that this Runyte version does not
/// understand.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UnsupportedModifiers {
    bits: u8,
}

impl UnsupportedModifiers {
    pub const fn bits(self) -> u8 {
        self.bits
    }
}

impl fmt::Display for UnsupportedModifiers {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "terminal key uses unsupported modifier bits 0x{:02x}",
            self.bits
        )
    }
}

impl Error for UnsupportedModifiers {}

/// Converts one terminal event when it represents editing input.
///
/// Key presses and repeats, bracketed-paste text, and pointer events are
/// converted into the owned input boundary. Other lifecycle events return
/// `None`.
pub fn convert_event(event: CrosstermEvent) -> Result<Option<InputEvent>, UnsupportedModifiers> {
    match event {
        CrosstermEvent::FocusGained | CrosstermEvent::FocusLost | CrosstermEvent::Resize(_, _) => {
            Ok(None)
        }
        CrosstermEvent::Mouse(event) => convert_mouse_event(event).map(|event| Some(event.into())),
        CrosstermEvent::Key(event) => convert_key_event(event).map(|stroke| stroke.map(Into::into)),
        CrosstermEvent::Paste(text) => Ok(Some(InputEvent::Text(text))),
    }
}

fn convert_mouse_event(event: CrosstermMouseEvent) -> Result<PointerEvent, UnsupportedModifiers> {
    let button = |button| match button {
        CrosstermMouseButton::Left => PointerButton::Left,
        CrosstermMouseButton::Middle => PointerButton::Middle,
        CrosstermMouseButton::Right => PointerButton::Right,
    };
    let kind = match event.kind {
        CrosstermMouseEventKind::Down(value) => PointerEventKind::Down(button(value)),
        CrosstermMouseEventKind::Up(value) => PointerEventKind::Up(button(value)),
        CrosstermMouseEventKind::Drag(value) => PointerEventKind::Drag(button(value)),
        CrosstermMouseEventKind::Moved => PointerEventKind::Moved,
        CrosstermMouseEventKind::ScrollUp => PointerEventKind::ScrollUp,
        CrosstermMouseEventKind::ScrollDown => PointerEventKind::ScrollDown,
        CrosstermMouseEventKind::ScrollLeft => PointerEventKind::ScrollLeft,
        CrosstermMouseEventKind::ScrollRight => PointerEventKind::ScrollRight,
    };
    Ok(PointerEvent {
        kind,
        column: event.column,
        row: event.row,
        modifiers: convert_modifiers(event.modifiers)?,
    })
}

/// Converts a key press or held-key repeat and rejects lifecycle-only input.
///
/// Enhanced keyboard protocols report pressing a modifier separately from the
/// character produced while it is held. The standalone report carries no
/// editor intent; admitting it would cancel a pending sequence such as
/// `Space W` before the shifted `W` arrives.
pub fn convert_key_event(
    event: CrosstermKeyEvent,
) -> Result<Option<KeyStroke>, UnsupportedModifiers> {
    match event.kind {
        CrosstermKeyEventKind::Press | CrosstermKeyEventKind::Repeat => {}
        CrosstermKeyEventKind::Release => return Ok(None),
    }
    if matches!(event.code, CrosstermKeyCode::Modifier(_)) {
        return Ok(None);
    }
    convert_key_stroke(event).map(Some)
}

fn convert_key_stroke(event: CrosstermKeyEvent) -> Result<KeyStroke, UnsupportedModifiers> {
    Ok(KeyStroke::new(
        convert_key_code(event.code),
        convert_modifiers(event.modifiers)?,
    ))
}

fn convert_modifiers(modifiers: CrosstermModifiers) -> Result<Modifiers, UnsupportedModifiers> {
    let known = CrosstermModifiers::SHIFT
        | CrosstermModifiers::CONTROL
        | CrosstermModifiers::ALT
        | CrosstermModifiers::SUPER
        | CrosstermModifiers::HYPER
        | CrosstermModifiers::META;
    let unknown = modifiers.bits() & !known.bits();
    if unknown != 0 {
        return Err(UnsupportedModifiers { bits: unknown });
    }

    let mut converted = Modifiers::NONE;
    if modifiers.contains(CrosstermModifiers::SHIFT) {
        converted.insert(Modifiers::SHIFT);
    }
    if modifiers.contains(CrosstermModifiers::CONTROL) {
        converted.insert(Modifiers::CONTROL);
    }
    if modifiers.contains(CrosstermModifiers::ALT) {
        converted.insert(Modifiers::ALT);
    }
    if modifiers.contains(CrosstermModifiers::SUPER) {
        converted.insert(Modifiers::SUPER);
    }
    if modifiers.contains(CrosstermModifiers::HYPER) {
        converted.insert(Modifiers::HYPER);
    }
    if modifiers.contains(CrosstermModifiers::META) {
        converted.insert(Modifiers::META);
    }
    Ok(converted)
}

fn convert_key_code(code: CrosstermKeyCode) -> KeyCode {
    match code {
        CrosstermKeyCode::Backspace => KeyCode::Backspace,
        CrosstermKeyCode::Enter => KeyCode::Enter,
        CrosstermKeyCode::Left => KeyCode::Left,
        CrosstermKeyCode::Right => KeyCode::Right,
        CrosstermKeyCode::Up => KeyCode::Up,
        CrosstermKeyCode::Down => KeyCode::Down,
        CrosstermKeyCode::Home => KeyCode::Home,
        CrosstermKeyCode::End => KeyCode::End,
        CrosstermKeyCode::PageUp => KeyCode::PageUp,
        CrosstermKeyCode::PageDown => KeyCode::PageDown,
        CrosstermKeyCode::Tab => KeyCode::Tab,
        CrosstermKeyCode::BackTab => KeyCode::BackTab,
        CrosstermKeyCode::Delete => KeyCode::Delete,
        CrosstermKeyCode::Insert => KeyCode::Insert,
        CrosstermKeyCode::F(number) => KeyCode::Function(number),
        CrosstermKeyCode::Char(character) => KeyCode::Char(character),
        CrosstermKeyCode::Null => KeyCode::Null,
        CrosstermKeyCode::Esc => KeyCode::Escape,
        CrosstermKeyCode::CapsLock => KeyCode::CapsLock,
        CrosstermKeyCode::ScrollLock => KeyCode::ScrollLock,
        CrosstermKeyCode::NumLock => KeyCode::NumLock,
        CrosstermKeyCode::PrintScreen => KeyCode::PrintScreen,
        CrosstermKeyCode::Pause => KeyCode::Pause,
        CrosstermKeyCode::Menu => KeyCode::Menu,
        CrosstermKeyCode::KeypadBegin => KeyCode::KeypadBegin,
        CrosstermKeyCode::Media(key) => KeyCode::Media(convert_media_key(key)),
        CrosstermKeyCode::Modifier(key) => KeyCode::Modifier(convert_modifier_key(key)),
    }
}

fn convert_media_key(key: CrosstermMediaKey) -> MediaKey {
    match key {
        CrosstermMediaKey::Play => MediaKey::Play,
        CrosstermMediaKey::Pause => MediaKey::Pause,
        CrosstermMediaKey::PlayPause => MediaKey::PlayPause,
        CrosstermMediaKey::Reverse => MediaKey::Reverse,
        CrosstermMediaKey::Stop => MediaKey::Stop,
        CrosstermMediaKey::FastForward => MediaKey::FastForward,
        CrosstermMediaKey::Rewind => MediaKey::Rewind,
        CrosstermMediaKey::TrackNext => MediaKey::TrackNext,
        CrosstermMediaKey::TrackPrevious => MediaKey::TrackPrevious,
        CrosstermMediaKey::Record => MediaKey::Record,
        CrosstermMediaKey::LowerVolume => MediaKey::LowerVolume,
        CrosstermMediaKey::RaiseVolume => MediaKey::RaiseVolume,
        CrosstermMediaKey::MuteVolume => MediaKey::MuteVolume,
    }
}

fn convert_modifier_key(key: CrosstermModifierKey) -> ModifierKey {
    match key {
        CrosstermModifierKey::LeftShift => ModifierKey::LeftShift,
        CrosstermModifierKey::LeftControl => ModifierKey::LeftControl,
        CrosstermModifierKey::LeftAlt => ModifierKey::LeftAlt,
        CrosstermModifierKey::LeftSuper => ModifierKey::LeftSuper,
        CrosstermModifierKey::LeftHyper => ModifierKey::LeftHyper,
        CrosstermModifierKey::LeftMeta => ModifierKey::LeftMeta,
        CrosstermModifierKey::RightShift => ModifierKey::RightShift,
        CrosstermModifierKey::RightControl => ModifierKey::RightControl,
        CrosstermModifierKey::RightAlt => ModifierKey::RightAlt,
        CrosstermModifierKey::RightSuper => ModifierKey::RightSuper,
        CrosstermModifierKey::RightHyper => ModifierKey::RightHyper,
        CrosstermModifierKey::RightMeta => ModifierKey::RightMeta,
        CrosstermModifierKey::IsoLevel3Shift => ModifierKey::IsoLevel3Shift,
        CrosstermModifierKey::IsoLevel5Shift => ModifierKey::IsoLevel5Shift,
    }
}

#[cfg(test)]
mod tests {
    use crossterm::event::{KeyEventState, MouseButton, MouseEvent, MouseEventKind};

    use super::*;

    fn press(code: CrosstermKeyCode, modifiers: CrosstermModifiers) -> CrosstermKeyEvent {
        CrosstermKeyEvent::new(code, modifiers)
    }

    #[test]
    fn every_plain_key_code_converts_without_fallback() {
        let cases = [
            (CrosstermKeyCode::Backspace, KeyCode::Backspace),
            (CrosstermKeyCode::Enter, KeyCode::Enter),
            (CrosstermKeyCode::Left, KeyCode::Left),
            (CrosstermKeyCode::Right, KeyCode::Right),
            (CrosstermKeyCode::Up, KeyCode::Up),
            (CrosstermKeyCode::Down, KeyCode::Down),
            (CrosstermKeyCode::Home, KeyCode::Home),
            (CrosstermKeyCode::End, KeyCode::End),
            (CrosstermKeyCode::PageUp, KeyCode::PageUp),
            (CrosstermKeyCode::PageDown, KeyCode::PageDown),
            (CrosstermKeyCode::Tab, KeyCode::Tab),
            (CrosstermKeyCode::BackTab, KeyCode::BackTab),
            (CrosstermKeyCode::Delete, KeyCode::Delete),
            (CrosstermKeyCode::Insert, KeyCode::Insert),
            (CrosstermKeyCode::F(24), KeyCode::Function(24)),
            (CrosstermKeyCode::Char('α'), KeyCode::Char('α')),
            (CrosstermKeyCode::Null, KeyCode::Null),
            (CrosstermKeyCode::Esc, KeyCode::Escape),
            (CrosstermKeyCode::CapsLock, KeyCode::CapsLock),
            (CrosstermKeyCode::ScrollLock, KeyCode::ScrollLock),
            (CrosstermKeyCode::NumLock, KeyCode::NumLock),
            (CrosstermKeyCode::PrintScreen, KeyCode::PrintScreen),
            (CrosstermKeyCode::Pause, KeyCode::Pause),
            (CrosstermKeyCode::Menu, KeyCode::Menu),
            (CrosstermKeyCode::KeypadBegin, KeyCode::KeypadBegin),
        ];
        for (native, owned) in cases {
            assert_eq!(
                convert_key_event(press(native, CrosstermModifiers::NONE)).unwrap(),
                Some(KeyStroke::plain(owned))
            );
        }
    }

    #[test]
    fn shifted_uppercase_character_reaches_the_owned_grammar_boundary_verbatim() {
        assert_eq!(
            convert_key_event(press(
                CrosstermKeyCode::Char('G'),
                CrosstermModifiers::SHIFT,
            ))
            .unwrap(),
            Some(KeyStroke::new(KeyCode::Char('G'), Modifiers::SHIFT))
        );
    }

    #[test]
    fn every_media_key_converts_without_fallback() {
        let cases = [
            (CrosstermMediaKey::Play, MediaKey::Play),
            (CrosstermMediaKey::Pause, MediaKey::Pause),
            (CrosstermMediaKey::PlayPause, MediaKey::PlayPause),
            (CrosstermMediaKey::Reverse, MediaKey::Reverse),
            (CrosstermMediaKey::Stop, MediaKey::Stop),
            (CrosstermMediaKey::FastForward, MediaKey::FastForward),
            (CrosstermMediaKey::Rewind, MediaKey::Rewind),
            (CrosstermMediaKey::TrackNext, MediaKey::TrackNext),
            (CrosstermMediaKey::TrackPrevious, MediaKey::TrackPrevious),
            (CrosstermMediaKey::Record, MediaKey::Record),
            (CrosstermMediaKey::LowerVolume, MediaKey::LowerVolume),
            (CrosstermMediaKey::RaiseVolume, MediaKey::RaiseVolume),
            (CrosstermMediaKey::MuteVolume, MediaKey::MuteVolume),
        ];
        for (native, owned) in cases {
            assert_eq!(
                convert_key_event(press(
                    CrosstermKeyCode::Media(native),
                    CrosstermModifiers::NONE,
                ))
                .unwrap(),
                Some(KeyStroke::plain(KeyCode::Media(owned)))
            );
        }
    }

    #[test]
    fn standalone_modifier_events_do_not_become_editor_input() {
        let cases = [
            (CrosstermModifierKey::LeftShift, ModifierKey::LeftShift),
            (CrosstermModifierKey::LeftControl, ModifierKey::LeftControl),
            (CrosstermModifierKey::LeftAlt, ModifierKey::LeftAlt),
            (CrosstermModifierKey::LeftSuper, ModifierKey::LeftSuper),
            (CrosstermModifierKey::LeftHyper, ModifierKey::LeftHyper),
            (CrosstermModifierKey::LeftMeta, ModifierKey::LeftMeta),
            (CrosstermModifierKey::RightShift, ModifierKey::RightShift),
            (
                CrosstermModifierKey::RightControl,
                ModifierKey::RightControl,
            ),
            (CrosstermModifierKey::RightAlt, ModifierKey::RightAlt),
            (CrosstermModifierKey::RightSuper, ModifierKey::RightSuper),
            (CrosstermModifierKey::RightHyper, ModifierKey::RightHyper),
            (CrosstermModifierKey::RightMeta, ModifierKey::RightMeta),
            (
                CrosstermModifierKey::IsoLevel3Shift,
                ModifierKey::IsoLevel3Shift,
            ),
            (
                CrosstermModifierKey::IsoLevel5Shift,
                ModifierKey::IsoLevel5Shift,
            ),
        ];
        for (native, owned) in cases {
            assert_eq!(
                convert_key_code(CrosstermKeyCode::Modifier(native)),
                KeyCode::Modifier(owned)
            );
            for kind in [
                CrosstermKeyEventKind::Press,
                CrosstermKeyEventKind::Repeat,
                CrosstermKeyEventKind::Release,
            ] {
                let event = CrosstermKeyEvent::new_with_kind(
                    CrosstermKeyCode::Modifier(native),
                    CrosstermModifiers::SHIFT,
                    kind,
                );
                assert_eq!(convert_key_event(event).unwrap(), None);
            }
        }
    }

    #[test]
    fn standalone_shift_does_not_enter_a_space_uppercase_sequence() {
        let events = [
            press(CrosstermKeyCode::Char(' '), CrosstermModifiers::NONE),
            press(
                CrosstermKeyCode::Modifier(CrosstermModifierKey::LeftShift),
                CrosstermModifiers::SHIFT,
            ),
            press(CrosstermKeyCode::Char('W'), CrosstermModifiers::SHIFT),
        ];
        let converted = events
            .into_iter()
            .filter_map(|event| convert_key_event(event).unwrap())
            .collect::<Vec<_>>();

        assert_eq!(
            converted,
            vec![
                KeyStroke::char(' '),
                KeyStroke::new(KeyCode::Char('W'), Modifiers::SHIFT),
            ]
        );
    }

    #[test]
    fn every_known_modifier_combination_is_mapped_explicitly() {
        for bits in 0..=CrosstermModifiers::all().bits() {
            let Some(native) = CrosstermModifiers::from_bits(bits) else {
                continue;
            };
            let converted = convert_key_event(press(CrosstermKeyCode::Char('x'), native))
                .unwrap()
                .unwrap();
            assert_eq!(converted.modifiers.bits(), bits);
        }
    }

    #[test]
    fn unknown_modifier_bits_are_rejected_instead_of_dropped() {
        let native = CrosstermModifiers::from_bits_retain(1 << 7);
        let error = convert_key_event(press(CrosstermKeyCode::Char('x'), native)).unwrap_err();
        assert_eq!(error.bits(), 1 << 7);
        assert_eq!(
            error.to_string(),
            "terminal key uses unsupported modifier bits 0x80"
        );
    }

    #[test]
    fn presses_and_repeats_become_editor_input_but_releases_do_not() {
        let state = KeyEventState::KEYPAD | KeyEventState::CAPS_LOCK | KeyEventState::NUM_LOCK;
        let pressed = CrosstermKeyEvent::new_with_kind_and_state(
            CrosstermKeyCode::Char('X'),
            CrosstermModifiers::SHIFT,
            CrosstermKeyEventKind::Press,
            state,
        );
        assert_eq!(
            convert_key_event(pressed).unwrap(),
            Some(KeyStroke::new(KeyCode::Char('X'), Modifiers::SHIFT))
        );

        let repeated = CrosstermKeyEvent::new_with_kind_and_state(
            CrosstermKeyCode::Enter,
            CrosstermModifiers::NONE,
            CrosstermKeyEventKind::Repeat,
            state,
        );
        assert_eq!(
            convert_key_event(repeated).unwrap(),
            Some(KeyStroke::plain(KeyCode::Enter))
        );

        let released = CrosstermKeyEvent::new_with_kind_and_state(
            CrosstermKeyCode::Enter,
            CrosstermModifiers::NONE,
            CrosstermKeyEventKind::Release,
            state,
        );
        assert_eq!(convert_key_event(released).unwrap(), None);
    }

    #[test]
    fn terminal_lifecycle_events_are_not_editor_input_and_pointer_is_owned() {
        let mouse = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 4,
            row: 7,
            modifiers: CrosstermModifiers::CONTROL,
        };
        for event in [
            CrosstermEvent::FocusGained,
            CrosstermEvent::FocusLost,
            CrosstermEvent::Resize(120, 40),
        ] {
            assert_eq!(convert_event(event).unwrap(), None);
        }
        assert_eq!(
            convert_event(CrosstermEvent::Mouse(mouse)).unwrap(),
            Some(InputEvent::Pointer(PointerEvent {
                kind: PointerEventKind::Down(PointerButton::Left),
                column: 4,
                row: 7,
                modifiers: Modifiers::CONTROL,
            }))
        );
    }

    #[test]
    fn key_events_are_wrapped_in_the_owned_input_event() {
        let event = CrosstermEvent::Key(press(
            CrosstermKeyCode::Char('i'),
            CrosstermModifiers::CONTROL | CrosstermModifiers::ALT,
        ));
        assert_eq!(
            convert_event(event).unwrap(),
            Some(InputEvent::Key(KeyStroke::new(
                KeyCode::Char('i'),
                Modifiers::CONTROL | Modifiers::ALT,
            )))
        );
    }

    #[test]
    fn bracketed_paste_is_one_literal_text_event() {
        let text = "i:quit\nα\t";
        assert_eq!(
            convert_event(CrosstermEvent::Paste(text.to_owned())).unwrap(),
            Some(InputEvent::Text(text.to_owned()))
        );
    }
}
