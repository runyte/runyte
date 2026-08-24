// SPDX-License-Identifier: MPL-2.0

//! Presentation-neutral editor input values.
//!
//! Frontends convert their native events into these types before input reaches
//! the editor. This module deliberately has no terminal-backend or protocol
//! serialization dependency: a terminal adapter, future GUI, and direct tests
//! should all be able to describe the same logical input.

use std::{fmt, ops};

/// One input delivered to an editing grammar.
#[derive(Clone, Eq, PartialEq)]
pub enum InputEvent {
    Key(KeyStroke),
    /// Literal text input, distinct from a sequence of bindable character
    /// keys. This is the boundary for paste, IME, and future frontend text
    /// composition.
    Text(String),
    /// Presentation-neutral pointer input. Frontends retain physical screen
    /// coordinates; the editor resolves them through the last prepared view.
    Pointer(PointerEvent),
}

impl From<KeyStroke> for InputEvent {
    fn from(stroke: KeyStroke) -> Self {
        Self::Key(stroke)
    }
}

impl From<String> for InputEvent {
    fn from(text: String) -> Self {
        Self::Text(text)
    }
}

impl From<&str> for InputEvent {
    fn from(text: &str) -> Self {
        Self::Text(text.to_owned())
    }
}

impl From<PointerEvent> for InputEvent {
    fn from(event: PointerEvent) -> Self {
        Self::Pointer(event)
    }
}

impl fmt::Debug for InputEvent {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Key(stroke) => formatter.debug_tuple("Key").field(stroke).finish(),
            // Pasted or composed text may contain private source material.
            // Keep diagnostics useful without making Debug an accidental log
            // of the text itself.
            Self::Text(text) => formatter
                .debug_struct("Text")
                .field("bytes", &text.len())
                .field("characters", &text.chars().count())
                .finish(),
            Self::Pointer(event) => formatter.debug_tuple("Pointer").field(event).finish(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PointerEvent {
    pub kind: PointerEventKind,
    pub column: u16,
    pub row: u16,
    pub modifiers: Modifiers,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PointerEventKind {
    Down(PointerButton),
    Up(PointerButton),
    Drag(PointerButton),
    Moved,
    ScrollUp,
    ScrollDown,
    ScrollLeft,
    ScrollRight,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PointerButton {
    Left,
    Middle,
    Right,
}

/// One key and the modifiers reported with it by a frontend.
///
/// The stored value is the raw Runyte-owned stroke. Binding lookup uses
/// [`Self::canonical_for_binding`] so overlays and macro recording can retain
/// the original modifiers while the registry follows its established Shift
/// rule for character keys.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct KeyStroke {
    pub code: KeyCode,
    pub modifiers: Modifiers,
}

impl KeyStroke {
    pub const fn new(code: KeyCode, modifiers: Modifiers) -> Self {
        Self { code, modifiers }
    }

    pub const fn char(value: char) -> Self {
        Self::new(KeyCode::Char(value), Modifiers::NONE)
    }

    pub const fn plain(code: KeyCode) -> Self {
        Self::new(code, Modifiers::NONE)
    }

    pub const fn ctrl(value: char) -> Self {
        Self::new(KeyCode::Char(value), Modifiers::CONTROL)
    }

    pub const fn alt(value: char) -> Self {
        Self::new(KeyCode::Char(value), Modifiers::ALT)
    }

    /// Returns the identity used by the keybinding registry.
    ///
    /// Terminal protocols generally encode Shift in the resulting character.
    /// Runyte therefore ignores Shift for character bindings while retaining
    /// the character and every other modifier exactly as supplied. In
    /// particular, this does not case-fold `Char('g')` into `Char('G')`.
    pub const fn canonical_for_binding(mut self) -> Self {
        if matches!(self.code, KeyCode::Char(_)) {
            self.modifiers.remove(Modifiers::SHIFT);
        }
        self
    }

    /// The stable, frontend-independent label used by hints and help.
    pub fn label(self) -> String {
        let modifiers = self.modifiers.label();
        let code = self.code.label();
        if modifiers.is_empty() {
            code
        } else {
            format!("{modifiers}-{code}")
        }
    }
}

impl fmt::Display for KeyStroke {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.pad(&self.label())
    }
}

/// Modifier flags attached to a [`KeyStroke`].
///
/// The bit layout is Runyte-owned. Frontend adapters must map their native
/// flags explicitly rather than passing backend bits through.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct Modifiers(u8);

impl Modifiers {
    pub const NONE: Self = Self(0);
    pub const SHIFT: Self = Self(1 << 0);
    pub const CONTROL: Self = Self(1 << 1);
    pub const ALT: Self = Self(1 << 2);
    pub const SUPER: Self = Self(1 << 3);
    pub const HYPER: Self = Self(1 << 4);
    pub const META: Self = Self(1 << 5);
    pub const ALL: Self = Self(
        Self::SHIFT.0
            | Self::CONTROL.0
            | Self::ALT.0
            | Self::SUPER.0
            | Self::HYPER.0
            | Self::META.0,
    );

    pub const fn from_bits(bits: u8) -> Option<Self> {
        if bits & !Self::ALL.0 == 0 {
            Some(Self(bits))
        } else {
            None
        }
    }

    pub const fn bits(self) -> u8 {
        self.0
    }

    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    pub const fn intersects(self, other: Self) -> bool {
        self.0 & other.0 != 0
    }

    pub const fn insert(&mut self, other: Self) {
        self.0 |= other.0;
    }

    pub const fn remove(&mut self, other: Self) {
        self.0 &= !other.0;
    }

    pub const fn with(mut self, other: Self) -> Self {
        self.insert(other);
        self
    }

    pub const fn without(mut self, other: Self) -> Self {
        self.remove(other);
        self
    }

    /// The stable modifier portion of a key label, without a trailing dash.
    pub fn label(self) -> String {
        let mut names = Vec::new();
        // Preserve the current key-hint ordering before appending the two
        // modifiers that the old presentation omitted.
        if self.contains(Self::CONTROL) {
            names.push("Ctrl");
        }
        if self.contains(Self::ALT) {
            names.push("Alt");
        }
        if self.contains(Self::SUPER) {
            names.push("Super");
        }
        if self.contains(Self::SHIFT) {
            names.push("Shift");
        }
        if self.contains(Self::HYPER) {
            names.push("Hyper");
        }
        if self.contains(Self::META) {
            names.push("Meta");
        }
        names.join("-")
    }
}

impl ops::BitOr for Modifiers {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self::Output {
        Self(self.0 | rhs.0)
    }
}

impl ops::BitOrAssign for Modifiers {
    fn bitor_assign(&mut self, rhs: Self) {
        self.insert(rhs);
    }
}

impl ops::BitAnd for Modifiers {
    type Output = Self;

    fn bitand(self, rhs: Self) -> Self::Output {
        Self(self.0 & rhs.0)
    }
}

impl ops::BitAndAssign for Modifiers {
    fn bitand_assign(&mut self, rhs: Self) {
        self.0 &= rhs.0;
    }
}

impl fmt::Display for Modifiers {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.pad(&self.label())
    }
}

/// A frontend-independent key code.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum KeyCode {
    Backspace,
    Enter,
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    PageUp,
    PageDown,
    Tab,
    BackTab,
    Delete,
    Insert,
    Function(u8),
    Char(char),
    Null,
    Escape,
    CapsLock,
    ScrollLock,
    NumLock,
    PrintScreen,
    Pause,
    Menu,
    KeypadBegin,
    Media(MediaKey),
    Modifier(ModifierKey),
}

impl KeyCode {
    /// The stable key-code portion of a key label.
    pub fn label(self) -> String {
        match self {
            Self::Backspace => "Backspace".to_owned(),
            Self::Enter => "Enter".to_owned(),
            Self::Left => "Left".to_owned(),
            Self::Right => "Right".to_owned(),
            Self::Up => "Up".to_owned(),
            Self::Down => "Down".to_owned(),
            Self::Home => "Home".to_owned(),
            Self::End => "End".to_owned(),
            Self::PageUp => "PageUp".to_owned(),
            Self::PageDown => "PageDown".to_owned(),
            Self::Tab => "Tab".to_owned(),
            Self::BackTab => "BackTab".to_owned(),
            Self::Delete => "Delete".to_owned(),
            Self::Insert => "Insert".to_owned(),
            Self::Function(number) => format!("F{number}"),
            Self::Char(' ') => "Space".to_owned(),
            Self::Char(character) => character.to_string(),
            Self::Null => "Null".to_owned(),
            Self::Escape => "Esc".to_owned(),
            Self::CapsLock => "CapsLock".to_owned(),
            Self::ScrollLock => "ScrollLock".to_owned(),
            Self::NumLock => "NumLock".to_owned(),
            Self::PrintScreen => "PrintScreen".to_owned(),
            Self::Pause => "Pause".to_owned(),
            Self::Menu => "Menu".to_owned(),
            Self::KeypadBegin => "KeypadBegin".to_owned(),
            Self::Media(key) => format!("Media({key})"),
            Self::Modifier(key) => format!("Modifier({key})"),
        }
    }
}

impl fmt::Display for KeyCode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.pad(&self.label())
    }
}

/// A media key reported by an enhanced keyboard protocol.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum MediaKey {
    Play,
    Pause,
    PlayPause,
    Reverse,
    Stop,
    FastForward,
    Rewind,
    TrackNext,
    TrackPrevious,
    Record,
    LowerVolume,
    RaiseVolume,
    MuteVolume,
}

impl MediaKey {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Play => "Play",
            Self::Pause => "Pause",
            Self::PlayPause => "PlayPause",
            Self::Reverse => "Reverse",
            Self::Stop => "Stop",
            Self::FastForward => "FastForward",
            Self::Rewind => "Rewind",
            Self::TrackNext => "TrackNext",
            Self::TrackPrevious => "TrackPrevious",
            Self::Record => "Record",
            Self::LowerVolume => "LowerVolume",
            Self::RaiseVolume => "RaiseVolume",
            Self::MuteVolume => "MuteVolume",
        }
    }
}

impl fmt::Display for MediaKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.pad(self.label())
    }
}

/// A standalone modifier key reported by an enhanced keyboard protocol.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ModifierKey {
    LeftShift,
    LeftControl,
    LeftAlt,
    LeftSuper,
    LeftHyper,
    LeftMeta,
    RightShift,
    RightControl,
    RightAlt,
    RightSuper,
    RightHyper,
    RightMeta,
    IsoLevel3Shift,
    IsoLevel5Shift,
}

impl ModifierKey {
    pub const fn label(self) -> &'static str {
        match self {
            Self::LeftShift => "LeftShift",
            Self::LeftControl => "LeftControl",
            Self::LeftAlt => "LeftAlt",
            Self::LeftSuper => "LeftSuper",
            Self::LeftHyper => "LeftHyper",
            Self::LeftMeta => "LeftMeta",
            Self::RightShift => "RightShift",
            Self::RightControl => "RightControl",
            Self::RightAlt => "RightAlt",
            Self::RightSuper => "RightSuper",
            Self::RightHyper => "RightHyper",
            Self::RightMeta => "RightMeta",
            Self::IsoLevel3Shift => "IsoLevel3Shift",
            Self::IsoLevel5Shift => "IsoLevel5Shift",
        }
    }
}

impl fmt::Display for ModifierKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.pad(self.label())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    #[test]
    fn every_modifier_combination_round_trips_through_owned_bits() {
        for bits in 0..=Modifiers::ALL.bits() {
            let modifiers = Modifiers::from_bits(bits).expect("all six-bit values are valid");
            assert_eq!(modifiers.bits(), bits);
        }
        assert_eq!(Modifiers::from_bits(1 << 6), None);
        assert_eq!(Modifiers::from_bits(1 << 7), None);
    }

    #[test]
    fn modifier_set_operations_retain_every_owned_flag() {
        let mut modifiers = Modifiers::CONTROL | Modifiers::ALT;
        modifiers.insert(Modifiers::HYPER | Modifiers::META);
        assert!(modifiers.contains(Modifiers::CONTROL | Modifiers::ALT));
        assert!(modifiers.intersects(Modifiers::HYPER | Modifiers::SUPER));
        assert!(!modifiers.contains(Modifiers::SHIFT));

        modifiers.remove(Modifiers::ALT | Modifiers::META);
        assert_eq!(modifiers, Modifiers::CONTROL | Modifiers::HYPER);
        assert_eq!(
            modifiers.with(Modifiers::SHIFT).without(Modifiers::CONTROL),
            Modifiers::SHIFT | Modifiers::HYPER
        );
    }

    #[test]
    fn binding_canonicalization_only_removes_shift_from_characters() {
        let raw = KeyStroke::new(
            KeyCode::Char('g'),
            Modifiers::SHIFT | Modifiers::ALT | Modifiers::META,
        );
        assert_eq!(
            raw.canonical_for_binding(),
            KeyStroke::new(KeyCode::Char('g'), Modifiers::ALT | Modifiers::META)
        );
        assert_eq!(
            raw.code,
            KeyCode::Char('g'),
            "canonicalization is not case folding"
        );
        assert!(
            raw.modifiers.contains(Modifiers::SHIFT),
            "the raw stroke is retained"
        );

        let back_tab = KeyStroke::new(KeyCode::BackTab, Modifiers::SHIFT);
        assert_eq!(back_tab.canonical_for_binding(), back_tab);
    }

    #[test]
    fn convenience_constructors_have_unambiguous_owned_identities() {
        let strokes = HashSet::from([
            KeyStroke::char('x'),
            KeyStroke::plain(KeyCode::Enter),
            KeyStroke::ctrl('x'),
            KeyStroke::alt('x'),
        ]);
        assert_eq!(strokes.len(), 4);
        assert_eq!(KeyStroke::char('x').modifiers, Modifiers::NONE);
    }

    #[test]
    fn key_labels_cover_every_non_parameterized_code() {
        let cases = [
            (KeyCode::Backspace, "Backspace"),
            (KeyCode::Enter, "Enter"),
            (KeyCode::Left, "Left"),
            (KeyCode::Right, "Right"),
            (KeyCode::Up, "Up"),
            (KeyCode::Down, "Down"),
            (KeyCode::Home, "Home"),
            (KeyCode::End, "End"),
            (KeyCode::PageUp, "PageUp"),
            (KeyCode::PageDown, "PageDown"),
            (KeyCode::Tab, "Tab"),
            (KeyCode::BackTab, "BackTab"),
            (KeyCode::Delete, "Delete"),
            (KeyCode::Insert, "Insert"),
            (KeyCode::Null, "Null"),
            (KeyCode::Escape, "Esc"),
            (KeyCode::CapsLock, "CapsLock"),
            (KeyCode::ScrollLock, "ScrollLock"),
            (KeyCode::NumLock, "NumLock"),
            (KeyCode::PrintScreen, "PrintScreen"),
            (KeyCode::Pause, "Pause"),
            (KeyCode::Menu, "Menu"),
            (KeyCode::KeypadBegin, "KeypadBegin"),
        ];
        for (code, expected) in cases {
            assert_eq!(code.label(), expected);
            assert_eq!(code.to_string(), expected);
        }
        assert_eq!(KeyCode::Function(12).label(), "F12");
        assert_eq!(KeyCode::Char(' ').label(), "Space");
        assert_eq!(KeyCode::Char('α').label(), "α");
    }

    #[test]
    fn media_and_standalone_modifier_labels_are_exhaustive() {
        let media = [
            MediaKey::Play,
            MediaKey::Pause,
            MediaKey::PlayPause,
            MediaKey::Reverse,
            MediaKey::Stop,
            MediaKey::FastForward,
            MediaKey::Rewind,
            MediaKey::TrackNext,
            MediaKey::TrackPrevious,
            MediaKey::Record,
            MediaKey::LowerVolume,
            MediaKey::RaiseVolume,
            MediaKey::MuteVolume,
        ];
        assert_eq!(
            media.map(MediaKey::label),
            [
                "Play",
                "Pause",
                "PlayPause",
                "Reverse",
                "Stop",
                "FastForward",
                "Rewind",
                "TrackNext",
                "TrackPrevious",
                "Record",
                "LowerVolume",
                "RaiseVolume",
                "MuteVolume",
            ]
        );

        let modifiers = [
            ModifierKey::LeftShift,
            ModifierKey::LeftControl,
            ModifierKey::LeftAlt,
            ModifierKey::LeftSuper,
            ModifierKey::LeftHyper,
            ModifierKey::LeftMeta,
            ModifierKey::RightShift,
            ModifierKey::RightControl,
            ModifierKey::RightAlt,
            ModifierKey::RightSuper,
            ModifierKey::RightHyper,
            ModifierKey::RightMeta,
            ModifierKey::IsoLevel3Shift,
            ModifierKey::IsoLevel5Shift,
        ];
        assert_eq!(
            modifiers.map(ModifierKey::label),
            [
                "LeftShift",
                "LeftControl",
                "LeftAlt",
                "LeftSuper",
                "LeftHyper",
                "LeftMeta",
                "RightShift",
                "RightControl",
                "RightAlt",
                "RightSuper",
                "RightHyper",
                "RightMeta",
                "IsoLevel3Shift",
                "IsoLevel5Shift",
            ]
        );
    }

    #[test]
    fn complete_stroke_labels_are_stable_and_width_aware() {
        let modifiers = Modifiers::CONTROL
            | Modifiers::ALT
            | Modifiers::SUPER
            | Modifiers::SHIFT
            | Modifiers::HYPER
            | Modifiers::META;
        let stroke = KeyStroke::new(KeyCode::Char('X'), modifiers);
        assert_eq!(stroke.label(), "Ctrl-Alt-Super-Shift-Hyper-Meta-X");
        assert_eq!(
            format!("{:<40}", KeyStroke::ctrl('w')),
            "Ctrl-w                                  "
        );
        assert_eq!(
            KeyStroke::plain(KeyCode::Media(MediaKey::PlayPause)).label(),
            "Media(PlayPause)"
        );
        assert_eq!(
            KeyStroke::plain(KeyCode::Modifier(ModifierKey::IsoLevel3Shift)).label(),
            "Modifier(IsoLevel3Shift)"
        );
    }

    #[test]
    fn text_debug_reports_shape_without_exposing_contents() {
        let input = InputEvent::Text("secret α".to_owned());
        let debug = format!("{input:?}");
        assert_eq!(debug, "Text { bytes: 9, characters: 8 }");
        assert!(!debug.contains("secret"));
        assert_eq!(
            InputEvent::from(KeyStroke::char('x')),
            InputEvent::Key(KeyStroke::char('x'))
        );
    }
}
