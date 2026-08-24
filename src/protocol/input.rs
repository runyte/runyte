// SPDX-License-Identifier: MPL-2.0

//! Wire-owned geometry and normalized input values.

use serde::{Deserialize, Serialize};

use crate::{
    app::FrameGeometry as CoreFrameGeometry,
    input::{
        InputEvent as CoreInputEvent, KeyCode as CoreKeyCode, KeyStroke as CoreKeyStroke,
        MediaKey as CoreMediaKey, ModifierKey as CoreModifierKey, Modifiers as CoreModifiers,
        PointerButton as CorePointerButton, PointerEvent as CorePointerEvent,
        PointerEventKind as CorePointerEventKind,
    },
    layout::Rect as CoreRect,
};

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct Rect {
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
}

impl From<CoreRect> for Rect {
    fn from(value: CoreRect) -> Self {
        Self {
            x: value.x,
            y: value.y,
            width: value.width,
            height: value.height,
        }
    }
}

impl From<Rect> for CoreRect {
    fn from(value: Rect) -> Self {
        Self {
            x: value.x,
            y: value.y,
            width: value.width,
            height: value.height,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct FrameGeometry {
    pub screen: Rect,
    pub editor: Rect,
    pub global_status_line: Rect,
    pub interaction_line: Rect,
}

impl From<CoreFrameGeometry> for FrameGeometry {
    fn from(value: CoreFrameGeometry) -> Self {
        Self {
            screen: value.screen.into(),
            editor: value.editor.into(),
            global_status_line: value.status.into(),
            interaction_line: value.message.into(),
        }
    }
}

impl From<FrameGeometry> for CoreFrameGeometry {
    fn from(value: FrameGeometry) -> Self {
        Self {
            screen: value.screen.into(),
            editor: value.editor.into(),
            status: value.global_status_line.into(),
            message: value.interaction_line.into(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum InputEvent {
    Key(KeyStroke),
    Text(String),
    Pointer(PointerEvent),
}

impl From<CoreInputEvent> for InputEvent {
    fn from(value: CoreInputEvent) -> Self {
        match value {
            CoreInputEvent::Key(key) => Self::Key(key.into()),
            CoreInputEvent::Text(text) => Self::Text(text),
            CoreInputEvent::Pointer(pointer) => Self::Pointer(pointer.into()),
        }
    }
}

impl From<InputEvent> for CoreInputEvent {
    fn from(value: InputEvent) -> Self {
        match value {
            InputEvent::Key(key) => Self::Key(key.into()),
            InputEvent::Text(text) => Self::Text(text),
            InputEvent::Pointer(pointer) => Self::Pointer(pointer.into()),
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PointerEvent {
    pub kind: PointerEventKind,
    pub column: u16,
    pub row: u16,
    pub modifiers: u8,
}

impl From<CorePointerEvent> for PointerEvent {
    fn from(value: CorePointerEvent) -> Self {
        Self {
            kind: value.kind.into(),
            column: value.column,
            row: value.row,
            modifiers: value.modifiers.bits(),
        }
    }
}

impl From<PointerEvent> for CorePointerEvent {
    fn from(value: PointerEvent) -> Self {
        Self {
            kind: value.kind.into(),
            column: value.column,
            row: value.row,
            modifiers: CoreModifiers::from_bits(value.modifiers)
                .expect("protocol pointer modifiers were validated"),
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
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

impl From<CorePointerEventKind> for PointerEventKind {
    fn from(value: CorePointerEventKind) -> Self {
        match value {
            CorePointerEventKind::Down(button) => Self::Down(button.into()),
            CorePointerEventKind::Up(button) => Self::Up(button.into()),
            CorePointerEventKind::Drag(button) => Self::Drag(button.into()),
            CorePointerEventKind::Moved => Self::Moved,
            CorePointerEventKind::ScrollUp => Self::ScrollUp,
            CorePointerEventKind::ScrollDown => Self::ScrollDown,
            CorePointerEventKind::ScrollLeft => Self::ScrollLeft,
            CorePointerEventKind::ScrollRight => Self::ScrollRight,
        }
    }
}

impl From<PointerEventKind> for CorePointerEventKind {
    fn from(value: PointerEventKind) -> Self {
        match value {
            PointerEventKind::Down(button) => Self::Down(button.into()),
            PointerEventKind::Up(button) => Self::Up(button.into()),
            PointerEventKind::Drag(button) => Self::Drag(button.into()),
            PointerEventKind::Moved => Self::Moved,
            PointerEventKind::ScrollUp => Self::ScrollUp,
            PointerEventKind::ScrollDown => Self::ScrollDown,
            PointerEventKind::ScrollLeft => Self::ScrollLeft,
            PointerEventKind::ScrollRight => Self::ScrollRight,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum PointerButton {
    Left,
    Middle,
    Right,
}

impl From<CorePointerButton> for PointerButton {
    fn from(value: CorePointerButton) -> Self {
        match value {
            CorePointerButton::Left => Self::Left,
            CorePointerButton::Middle => Self::Middle,
            CorePointerButton::Right => Self::Right,
        }
    }
}

impl From<PointerButton> for CorePointerButton {
    fn from(value: PointerButton) -> Self {
        match value {
            PointerButton::Left => Self::Left,
            PointerButton::Middle => Self::Middle,
            PointerButton::Right => Self::Right,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct KeyStroke {
    pub code: KeyCode,
    pub modifiers: u8,
}

impl From<CoreKeyStroke> for KeyStroke {
    fn from(value: CoreKeyStroke) -> Self {
        Self {
            code: value.code.into(),
            modifiers: value.modifiers.bits(),
        }
    }
}

impl From<KeyStroke> for CoreKeyStroke {
    fn from(value: KeyStroke) -> Self {
        Self {
            code: value.code.into(),
            modifiers: CoreModifiers::from_bits(value.modifiers)
                .expect("protocol key modifiers were validated"),
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
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

macro_rules! convert_key_code {
    ($value:expr, $source:ident, $target:ident) => {
        match $value {
            $source::Backspace => $target::Backspace,
            $source::Enter => $target::Enter,
            $source::Left => $target::Left,
            $source::Right => $target::Right,
            $source::Up => $target::Up,
            $source::Down => $target::Down,
            $source::Home => $target::Home,
            $source::End => $target::End,
            $source::PageUp => $target::PageUp,
            $source::PageDown => $target::PageDown,
            $source::Tab => $target::Tab,
            $source::BackTab => $target::BackTab,
            $source::Delete => $target::Delete,
            $source::Insert => $target::Insert,
            $source::Function(value) => $target::Function(value),
            $source::Char(value) => $target::Char(value),
            $source::Null => $target::Null,
            $source::Escape => $target::Escape,
            $source::CapsLock => $target::CapsLock,
            $source::ScrollLock => $target::ScrollLock,
            $source::NumLock => $target::NumLock,
            $source::PrintScreen => $target::PrintScreen,
            $source::Pause => $target::Pause,
            $source::Menu => $target::Menu,
            $source::KeypadBegin => $target::KeypadBegin,
            $source::Media(value) => $target::Media(value.into()),
            $source::Modifier(value) => $target::Modifier(value.into()),
        }
    };
}

impl From<CoreKeyCode> for KeyCode {
    fn from(value: CoreKeyCode) -> Self {
        convert_key_code!(value, CoreKeyCode, KeyCode)
    }
}

impl From<KeyCode> for CoreKeyCode {
    fn from(value: KeyCode) -> Self {
        convert_key_code!(value, KeyCode, CoreKeyCode)
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
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

macro_rules! convert_unit_enum {
    ($value:expr, $source:ident, $target:ident, [$($variant:ident),+ $(,)?]) => {
        match $value { $($source::$variant => $target::$variant),+ }
    };
}

impl From<CoreMediaKey> for MediaKey {
    fn from(value: CoreMediaKey) -> Self {
        convert_unit_enum!(
            value,
            CoreMediaKey,
            MediaKey,
            [
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
            ]
        )
    }
}

impl From<MediaKey> for CoreMediaKey {
    fn from(value: MediaKey) -> Self {
        convert_unit_enum!(
            value,
            MediaKey,
            CoreMediaKey,
            [
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
            ]
        )
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
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

impl From<CoreModifierKey> for ModifierKey {
    fn from(value: CoreModifierKey) -> Self {
        convert_unit_enum!(
            value,
            CoreModifierKey,
            ModifierKey,
            [
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
            ]
        )
    }
}

impl From<ModifierKey> for CoreModifierKey {
    fn from(value: ModifierKey) -> Self {
        convert_unit_enum!(
            value,
            ModifierKey,
            CoreModifierKey,
            [
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
            ]
        )
    }
}
