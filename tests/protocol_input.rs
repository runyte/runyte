// SPDX-License-Identifier: MPL-2.0

use runyte::{
    app::FrameGeometry,
    input::{
        InputEvent, KeyCode, KeyStroke, MediaKey, ModifierKey, Modifiers, PointerButton,
        PointerEvent, PointerEventKind,
    },
    layout::Rect,
};

fn round_trip(event: InputEvent) {
    let wire: runyte::protocol::InputEvent = event.clone().into();
    let encoded = serde_json::to_string(&wire).unwrap();
    let decoded: runyte::protocol::InputEvent = serde_json::from_str(&encoded).unwrap();
    let decoded: InputEvent = decoded.into();
    assert_eq!(decoded, event);
}

#[test]
fn every_key_identity_round_trips_through_the_wire_boundary() {
    let mut codes = vec![
        KeyCode::Backspace,
        KeyCode::Enter,
        KeyCode::Left,
        KeyCode::Right,
        KeyCode::Up,
        KeyCode::Down,
        KeyCode::Home,
        KeyCode::End,
        KeyCode::PageUp,
        KeyCode::PageDown,
        KeyCode::Tab,
        KeyCode::BackTab,
        KeyCode::Delete,
        KeyCode::Insert,
        KeyCode::Function(24),
        KeyCode::Char('ż'),
        KeyCode::Null,
        KeyCode::Escape,
        KeyCode::CapsLock,
        KeyCode::ScrollLock,
        KeyCode::NumLock,
        KeyCode::PrintScreen,
        KeyCode::Pause,
        KeyCode::Menu,
        KeyCode::KeypadBegin,
    ];
    codes.extend(
        [
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
        ]
        .map(KeyCode::Media),
    );
    codes.extend(
        [
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
        ]
        .map(KeyCode::Modifier),
    );

    for code in codes {
        round_trip(InputEvent::Key(KeyStroke::new(code, Modifiers::ALL)));
    }
}

#[test]
fn every_pointer_identity_and_literal_text_round_trip_through_the_wire_boundary() {
    let kinds = [
        PointerEventKind::Down(PointerButton::Left),
        PointerEventKind::Down(PointerButton::Middle),
        PointerEventKind::Down(PointerButton::Right),
        PointerEventKind::Up(PointerButton::Left),
        PointerEventKind::Up(PointerButton::Middle),
        PointerEventKind::Up(PointerButton::Right),
        PointerEventKind::Drag(PointerButton::Left),
        PointerEventKind::Drag(PointerButton::Middle),
        PointerEventKind::Drag(PointerButton::Right),
        PointerEventKind::Moved,
        PointerEventKind::ScrollUp,
        PointerEventKind::ScrollDown,
        PointerEventKind::ScrollLeft,
        PointerEventKind::ScrollRight,
    ];
    for kind in kinds {
        round_trip(InputEvent::Pointer(PointerEvent {
            kind,
            column: 321,
            row: 123,
            modifiers: Modifiers::SHIFT | Modifiers::CONTROL | Modifiers::META,
        }));
    }
    round_trip(InputEvent::Text("paste ż\n🙂".to_owned()));
}

#[test]
fn frame_geometry_round_trips_without_renaming_core_regions() {
    let geometry = FrameGeometry {
        screen: Rect {
            x: 1,
            y: 2,
            width: 120,
            height: 40,
        },
        editor: Rect {
            x: 3,
            y: 4,
            width: 100,
            height: 32,
        },
        status: Rect {
            x: 3,
            y: 36,
            width: 100,
            height: 1,
        },
        message: Rect {
            x: 3,
            y: 37,
            width: 100,
            height: 1,
        },
    };
    let wire: runyte::protocol::FrameGeometry = geometry.into();
    let encoded = serde_json::to_string(&wire).unwrap();

    assert!(encoded.contains("global_status_line"));
    assert!(encoded.contains("interaction_line"));
    let decoded: runyte::protocol::FrameGeometry = serde_json::from_str(&encoded).unwrap();
    let decoded: FrameGeometry = decoded.into();
    assert_eq!(decoded, geometry);
}
