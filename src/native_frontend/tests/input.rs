// SPDX-License-Identifier: MPL-2.0
use super::{mouse_event, translate_key};
use crossterm::event::{KeyCode, KeyModifiers};
use gpui::{Keystroke, point, px};

#[test]
fn native_strokes_preserve_modal_sequences_and_modifiers() {
    for (spelling, code, mods) in [
        ("ctrl-\\", KeyCode::Char('\\'), KeyModifiers::CONTROL),
        ("alt-v", KeyCode::Char('v'), KeyModifiers::ALT),
        ("ctrl-w", KeyCode::Char('w'), KeyModifiers::CONTROL),
        ("shift-tab", KeyCode::BackTab, KeyModifiers::SHIFT),
        ("shift-left", KeyCode::Left, KeyModifiers::SHIFT),
        ("f12", KeyCode::F(12), KeyModifiers::NONE),
        ("escape", KeyCode::Esc, KeyModifiers::NONE),
        ("space", KeyCode::Char(' '), KeyModifiers::NONE),
    ] {
        let event = translate_key(&Keystroke::parse(spelling).unwrap()).unwrap();
        assert_eq!(event.code, code, "{spelling}");
        assert_eq!(event.modifiers, mods, "{spelling}");
    }
    for (physical, produced) in [("g", "G"), (";", ":"), ("/", "?"), ("1", "!"), ("a", "ą")] {
        let key = Keystroke {
            key: physical.into(),
            key_char: Some(produced.into()),
            modifiers: gpui::Modifiers {
                shift: true,
                ..Default::default()
            },
        };
        assert_eq!(
            translate_key(&key).unwrap().code,
            KeyCode::Char(produced.chars().next().unwrap())
        );
    }
}

#[test]
fn native_pointer_uses_the_same_cell_geometry_as_rendering() {
    let event = mouse_event(
        point(px(93.), px(67.)),
        crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Left),
        gpui::Modifiers::default(),
    );
    let Some(runyte::input::InputEvent::Pointer(event)) =
        runyte::tui::input::convert_event(event).unwrap()
    else {
        panic!("pointer")
    };
    assert_eq!((event.column, event.row), (10, 3));
}

#[tokio::test]
async fn queued_native_input_keeps_its_painted_frame_and_cannot_confirm_unseen_plans() {
    use super::{Events, NativeInput};
    use crossterm::event::{Event, KeyEvent};
    use runyte::{
        app::{App, FrameGeometry, FsConfirmation, FsConfirmationOrigin},
        config::Config,
        fs_plan::{DesiredEntry, DirectorySnapshot, EntryKind, FsPlan},
        workspace::WorkspaceHost,
    };
    let root = tempfile::tempdir().unwrap();
    let app = App::new_in_project(Config::default(), None, root.path()).unwrap();
    let mut host = WorkspaceHost::new(app);
    host.defer_frontend_presentation(true);
    let first = host.prepare_frame(FrameGeometry::default()).id;
    let (send, events) = tokio::sync::mpsc::channel(8);
    let mut events = Events::Native {
        events,
        presented: None,
        presentation_only: false,
    };
    let enter = Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    send.send(NativeInput {
        event: enter.clone(),
        presented: Some(first),
        presentation_only: false,
    })
    .await
    .unwrap();
    let plan = FsPlan::build(
        root.path().to_path_buf(),
        DirectorySnapshot::read(root.path()).unwrap(),
        vec![DesiredEntry::create("approved.txt", EntryKind::File)],
    )
    .unwrap();
    host.app_mut().fs_confirmation = Some(FsConfirmation {
        origin: FsConfirmationOrigin::Explorer {
            buffer: host.app().active().buffer,
        },
        plan,
        selected: 0,
    });
    let confirmation = host.prepare_frame(FrameGeometry::default()).id;
    let input = runyte::tui::input::convert_event(events.next().await.unwrap().unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(events.presented_frame(Some(confirmation)), Some(first));
    assert!(!events.accepts_input(&host, &input, false));
    if events.accepts_input(&host, &input, false) {
        host.execute_frontend_input(input.clone(), false).unwrap();
    }
    assert!(!root.path().join("approved.txt").exists());
    assert!(host.app().fs_confirmation.is_some());
    send.send(NativeInput {
        event: enter,
        presented: Some(confirmation),
        presentation_only: false,
    })
    .await
    .unwrap();
    events.next().await.unwrap().unwrap();
    assert!(events.accepts_input(&host, &input, false));
    assert!(
        !events.accepts_input(&host, &input, true),
        "unpublished live changes must not inherit the prepared frame"
    );
    host.execute_frontend_input(input.clone(), false).unwrap();
    assert!(root.path().join("approved.txt").exists());
    // Ordinary queued editing must not depend on the redraw rate.
    host.prepare_frame(FrameGeometry::default());
    assert!(events.accepts_input(&host, &input, false));
    send.send(NativeInput {
        event: Event::Resize(80, 24),
        presented: None,
        presentation_only: false,
    })
    .await
    .unwrap();
    events.next().await.unwrap().unwrap();
    assert_eq!(events.presented_frame(host.current_frame_id()), None);
    assert!(!events.is_presentation_acknowledgement());
    send.send(NativeInput {
        event: Event::FocusGained,
        presented: host.current_frame_id(),
        presentation_only: true,
    })
    .await
    .unwrap();
    events.next().await.unwrap().unwrap();
    assert!(events.is_presentation_acknowledgement());
    assert_eq!(events.presented_frame(None), host.current_frame_id());
    drop(send);
    assert!(events.next().await.is_none());
    assert!(!events.is_presentation_acknowledgement());
}

#[test]
fn ordinary_native_editing_remains_queued_across_unpainted_frames() {
    use super::Events;
    use runyte::{
        app::{App, FrameGeometry},
        config::Config,
        input::{InputEvent, KeyStroke},
        workspace::WorkspaceHost,
    };
    let root = tempfile::tempdir().unwrap();
    let mut host =
        WorkspaceHost::new(App::new_in_project(Config::default(), None, root.path()).unwrap());
    host.defer_frontend_presentation(true);
    let first = host.prepare_frame(FrameGeometry::default()).id;
    let (_send, events) = tokio::sync::mpsc::channel(1);
    let events = Events::Native {
        events,
        presented: Some(first),
        presentation_only: false,
    };
    host.prepare_frame(FrameGeometry::default());
    for event in [
        InputEvent::Text("typing".into()),
        InputEvent::Key(KeyStroke::parse("j").unwrap()),
    ] {
        assert!(events.accepts_input(&host, &event, true));
    }
}

#[test]
fn retained_media_preserves_overlay_pixels_but_blocks_hidden_row_pointer_input() {
    let body = runyte::layout::Rect {
        x: 1,
        y: 1,
        width: 30,
        height: 20,
    };
    let overlay = runyte::layout::Rect {
        x: 5,
        y: 15,
        width: 20,
        height: 5,
    };
    let frame = super::FrameData {
        id: None,
        cells: ratatui::buffer::Buffer::empty(ratatui::layout::Rect::new(0, 0, 40, 30)),
        media: vec![super::MediaPane {
            pane: 0,
            path: "pages.pdf".into(),
            page: 2,
            body,
        }],
        cursor: None,
        overlays: vec![overlay],
        media_input: false,
        metadata_paths: Vec::new(),
    };
    assert!(frame.under_media(2, 2));
    assert!(
        !frame.under_media(6, 16),
        "overlay cells must paint above image"
    );
    assert!(!frame.under_media(35, 2));
    let mask = super::MediaInputMask {
        blocked: vec![body],
    };
    let point = |x: f32, y: f32| {
        gpui::point(
            gpui::px(x * super::CELL_WIDTH),
            gpui::px(y * super::CELL_HEIGHT),
        )
    };
    assert!(mask.blocks(point(2., 2.)));
    assert!(
        mask.blocks(point(6., 16.)),
        "hints cannot select underlying page rows"
    );
    assert!(!mask.blocks(point(35., 2.)));
}
