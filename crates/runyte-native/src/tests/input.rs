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
        ("cmd-c", KeyCode::Char('c'), KeyModifiers::SUPER),
        ("cmd-v", KeyCode::Char('v'), KeyModifiers::SUPER),
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
        super::CellMetrics::default(),
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
    let (send, events) = super::input_queue::channel(8);
    let mut events = Events::Native {
        attachment: std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0)),
        events,
        batch: 0,
        presented: None,
        presentation_only: false,
        routing_serial: 0,
    };
    let enter = Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    send.send(NativeInput {
        attachment: 0,
        event: enter.clone(),
        presented: Some(first),
        presentation_only: false,
        routing_serial: 0,
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
        attachment: 0,
        event: enter,
        presented: Some(confirmation),
        presentation_only: false,
        routing_serial: 0,
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
        attachment: 0,
        event: Event::Resize(80, 24),
        presented: None,
        presentation_only: false,
        routing_serial: 0,
    })
    .await
    .unwrap();
    events.next().await.unwrap().unwrap();
    assert_eq!(events.presented_frame(host.current_frame_id()), None);
    assert!(!events.is_presentation_acknowledgement());
    send.send(NativeInput {
        attachment: 0,
        event: Event::FocusGained,
        presented: host.current_frame_id(),
        presentation_only: true,
        routing_serial: 0,
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
    let (_send, events) = super::input_queue::channel(1);
    let events = Events::Native {
        attachment: std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0)),
        events,
        batch: 0,
        presented: Some(first),
        presentation_only: false,
        routing_serial: 0,
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
        attachment: 0,
        background: super::FALLBACK_BACKGROUND,
        foreground: super::FALLBACK_FOREGROUND,
        id: None,
        cells: ratatui::buffer::Buffer::empty(ratatui::layout::Rect::new(0, 0, 40, 30)).into(),
        previews: Vec::new(),
        media: vec![super::MediaPane {
            pane: 0,
            path: "pages.pdf".into(),
            page: 2,
            body,
        }],
        cursor: None,
        overlays: vec![overlay],
        media_input: false,
        routing_serial: 0,
        metadata_paths: Vec::new(),
    };
    assert!(frame.under_media(2, 2));
    assert!(
        !frame.under_media(6, 16),
        "overlay cells must paint above image"
    );
    assert!(!frame.under_media(35, 2));
    let mask = super::MediaInputMask {
        metrics: super::CellMetrics::default(),
        blocked: vec![body],
    };
    let point = |x: f32, y: f32| {
        gpui::point(
            gpui::px(x * super::CellMetrics::default().width),
            gpui::px(y * super::CellMetrics::default().height),
        )
    };
    assert!(mask.blocks(point(2., 2.)));
    assert!(
        mask.blocks(point(6., 16.)),
        "hints cannot select underlying page rows"
    );
    assert!(!mask.blocks(point(35., 2.)));
}

#[tokio::test]
async fn attachment_handoff_keeps_window_resize_but_drops_old_document_input() {
    use super::{Events, NativeInput};
    use crossterm::event::{Event, KeyEvent};
    let (send, receiver) = super::input_queue::channel(8);
    let attachment = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(2));
    let mut events = Events::Native {
        attachment,
        events: receiver,
        batch: 0,
        presented: None,
        presentation_only: false,
        routing_serial: 0,
    };
    for (generation, event, presentation_only) in [
        (
            1,
            Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
            false,
        ),
        (1, Event::FocusGained, true),
        (1, Event::Resize(160, 50), false),
        (
            2,
            Event::Key(KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE)),
            false,
        ),
    ] {
        send.send(NativeInput {
            attachment: generation,
            event,
            presented: Some(runyte::protocol::FrameId::from_raw(7).into()),
            presentation_only,
            routing_serial: 0,
        })
        .await
        .unwrap();
    }
    drop(send);
    assert_eq!(
        events.next().await.unwrap().unwrap(),
        Event::Resize(160, 50)
    );
    assert!(!events.is_presentation_acknowledgement());
    assert_eq!(
        events.next().await.unwrap().unwrap(),
        Event::Key(KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE))
    );
    assert!(events.next().await.is_none());
}

#[test]
fn media_actions_wait_for_their_visual_frame_and_do_not_cross_attachments() {
    use super::{FrameData, PendingMediaRequest, ready_media_requests};
    use runyte::{
        media::{ViewAction, ViewRequest},
        protocol::FrameId,
    };
    let mut frame = FrameData {
        attachment: 2,
        background: super::FALLBACK_BACKGROUND,
        foreground: super::FALLBACK_FOREGROUND,
        id: Some(FrameId::from_raw(10).into()),
        cells: ratatui::buffer::Buffer::empty(ratatui::layout::Rect::new(0, 0, 80, 24)).into(),
        previews: Vec::new(),
        media: Vec::new(),
        cursor: None,
        overlays: Vec::new(),
        media_input: true,
        routing_serial: 0,
        metadata_paths: Vec::new(),
    };
    let request = || ViewRequest {
        pane: 1,
        path: "pages.pdf".into(),
        page: 2,
        action: ViewAction::ZoomIn,
    };
    let mut queue = std::collections::VecDeque::from([
        PendingMediaRequest {
            attachment: 1,
            frame: FrameId::from_raw(11).into(),
            request: request(),
        },
        PendingMediaRequest {
            attachment: 2,
            frame: FrameId::from_raw(11).into(),
            request: request(),
        },
    ]);
    assert!(ready_media_requests(&mut queue, None, 2).is_empty());
    assert_eq!(queue.len(), 1, "old attachment must be discarded");
    assert!(ready_media_requests(&mut queue, Some(&frame), 2).is_empty());
    assert_eq!(
        queue.len(),
        1,
        "page-buffer frame cannot consume preview action"
    );
    // A coalesced replacement can skip the exact frame but still releases FIFO actions.
    frame.id = Some(FrameId::from_raw(12).into());
    frame.media.push(super::MediaPane {
        pane: 1,
        path: "pages.pdf".into(),
        page: 2,
        body: runyte::layout::Rect::default(),
    });
    let ready = ready_media_requests(&mut queue, Some(&frame), 2);
    assert_eq!(ready.len(), 1);
    assert_eq!(ready[0].action, ViewAction::ZoomIn);
    assert!(queue.is_empty());
}

#[test]
fn resized_font_scales_pointer_and_media_hit_testing_together() {
    for font_size in [8, 15, 30, 48] {
        let metrics = super::CellMetrics::new(font_size);
        let position = point(px(10.5 * metrics.width), px(3.5 * metrics.height));
        let crossterm::event::Event::Mouse(event) = mouse_event(
            metrics,
            position,
            crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Left),
            gpui::Modifiers::default(),
        ) else {
            panic!("mouse event")
        };
        assert_eq!((event.column, event.row), (10, 3));
        let mask = super::MediaInputMask {
            metrics,
            blocked: vec![runyte::layout::Rect {
                x: 10,
                y: 3,
                width: 1,
                height: 1,
            }],
        };
        assert!(mask.blocks(position));
        assert!(!mask.blocks(point(px(11.5 * metrics.width), position.y)));
        assert_eq!(metrics.font_size, font_size as f32);
    }
    assert_eq!(super::CellMetrics::new(0).font_size, 8.);
    assert_eq!(super::CellMetrics::new(usize::MAX).font_size, 48.);
}

#[test]
fn fractional_scroll_accumulates_into_whole_host_events() {
    use super::{CellMetrics, ScrollAccumulator, WHEEL_LINES_PER_NOTCH};
    use crossterm::event::MouseEventKind::*;
    use gpui::{ScrollDelta, point, px};
    use std::time::{Duration, Instant};
    let metrics = CellMetrics::new(15);
    let lines = |y: f32| ScrollAccumulator::delta(ScrollDelta::Lines(point(0., y)), metrics);
    let start = Instant::now();
    let at = |ms: u64| start + Duration::from_millis(ms);
    let mut scroll = ScrollAccumulator::default();
    // One wheel notch is exactly one host event.
    assert_eq!(
        scroll.push(lines(WHEEL_LINES_PER_NOTCH), at(0)),
        Some((ScrollUp, 1))
    );
    assert_eq!(
        scroll.push(lines(-WHEEL_LINES_PER_NOTCH), at(10)),
        Some((ScrollDown, 1))
    );
    // Thirty touchpad steps of 2 px (0.1 lines at 20 px) are one event of three
    // lines, not thirty.
    let pixels = ScrollAccumulator::delta(ScrollDelta::Pixels(point(px(0.), px(2.))), metrics);
    let events: usize = (0..30)
        .filter_map(|step| scroll.push(pixels, at(20 + step)))
        .map(|(kind, count)| {
            assert_eq!(kind, ScrollUp);
            count
        })
        .sum();
    assert_eq!(events, 1);
    let events = |x: f32, y: f32| (point(x, y), y.abs() >= x.abs());
    // Reversing discards the remainder of the opposite direction.
    assert_eq!(scroll.push(events(0., 0.6), at(60)), None);
    assert_eq!(scroll.push(events(0., -0.3), at(70)), None);
    assert_eq!(scroll.push(events(0., -0.7), at(80)), Some((ScrollDown, 1)));
    // A pause ends the gesture: its fraction does not complete a later nudge.
    assert_eq!(scroll.push(events(0., 0.9), at(90)), None);
    assert_eq!(scroll.push(events(0., 0.2), at(1_000)), None);
    // The dominant axis wins, so vertical jitter does not block a sideways swipe.
    assert_eq!(
        scroll.push(events(2., 0.1), at(1_010)),
        Some((ScrollLeft, 2))
    );
    assert_eq!(
        scroll.push(events(-1., 0.), at(1_020)),
        Some((ScrollRight, 1))
    );
    assert_eq!(scroll.push(events(0., 0.), at(1_030)), None);
    // The axis is judged in pixels: a vertical swipe drifting sideways by half
    // its height stays vertical even though columns are narrower than rows.
    let drift = ScrollAccumulator::delta(ScrollDelta::Pixels(point(px(30.), px(60.))), metrics);
    assert!(drift.1 && drift.0.x > drift.0.y);
    // A single event never sends more than twenty scrolls.
    assert_eq!(
        scroll.push(events(0., -100.), at(1_040)),
        Some((ScrollDown, 20))
    );
}

#[test]
fn frames_keep_the_metrics_they_were_laid_out_for() {
    use super::{CellMetrics, painted_metrics};
    use gpui::{px, size};
    let old = CellMetrics::new(15);
    let new = CellMetrics::new(20);
    let viewport = size(px(1080.), px(800.));
    assert_eq!(old.grid(viewport), (120, 40));
    let requested = new.grid(viewport);
    assert_eq!(requested, (90, 29));
    // After a font change the previous frame keeps its own metrics.
    assert_eq!(painted_metrics(old, new, Some((120, 40)), requested), old);
    // The frame laid out for the requested grid switches to the new metrics.
    assert_eq!(painted_metrics(old, new, Some(requested), requested), new);
    // Before any frame, and when the grid did not change, nothing waits.
    assert_eq!(painted_metrics(old, new, None, requested), new);
    assert_eq!(painted_metrics(old, new, Some(requested), requested), new);
    // A resize at unchanged metrics paints the stale frame at those metrics.
    assert_eq!(painted_metrics(old, old, Some((120, 40)), (130, 45)), old);
}

#[test]
fn rapid_font_changes_preserve_intermediate_frame_metrics_and_bound_history() {
    use super::{CellMetrics, GeometryRequests, painted_metrics};
    let mut history = GeometryRequests::default();
    let viewport = gpui::size(px(1080.), px(800.));
    for font in [15, 16, 17] {
        let metrics = CellMetrics::new(font);
        history.record(metrics.grid(viewport), metrics);
    }
    let middle = CellMetrics::new(16);
    let latest = CellMetrics::new(17);
    let received = history.metrics(middle.grid(viewport)).unwrap();
    assert_eq!(
        painted_metrics(
            received,
            latest,
            Some(middle.grid(viewport)),
            latest.grid(viewport)
        ),
        middle
    );
    assert_eq!(history.metrics(latest.grid(viewport)), Some(latest));
    // A same-grid request may adopt new metrics immediately: layout is unchanged.
    history.record(middle.grid(viewport), latest);
    assert_eq!(history.metrics(middle.grid(viewport)), Some(latest));
    for width in 200..300 {
        history.record((width, 50), latest);
    }
    assert_eq!(history.0.len(), 64);
    assert!(history.metrics(middle.grid(viewport)).is_none());
    assert_eq!(history.metrics((299, 50)), Some(latest));
}

#[test]
fn attached_snapshot_renders_at_host_geometry_during_resize() {
    use ratatui::{Terminal, layout::Rect};
    let root = tempfile::tempdir().unwrap();
    let app =
        runyte::app::App::new_in_project(runyte::config::Config::default(), None, root.path())
            .unwrap();
    let mut host = runyte::workspace::WorkspaceHost::new(app);
    let snapshot = host.prepare_frame(runyte::ui::frame_geometry(Rect::new(0, 0, 120, 40)));
    let mut terminal = Terminal::new(super::grid::GridBackend::new(80, 24)).unwrap();
    super::draw_native_grid(
        &mut terminal,
        (80, 24),
        Some(snapshot.editor.geometry.screen),
        |frame| {
            runyte::ui::render_host_frame_exact_colors_for_test(frame, &snapshot);
        },
    )
    .unwrap();
    assert_eq!(terminal.backend().snapshot().area, Rect::new(0, 0, 120, 40));
    assert!(
        terminal.backend().snapshot().rows[38]
            .iter()
            .any(|cell| cell.symbol() != " ")
    );
    super::draw_native_grid(&mut terminal, (80, 24), None, |_| {}).unwrap();
    assert_eq!(terminal.backend().snapshot().area, Rect::new(0, 0, 80, 24));
}

#[tokio::test]
async fn queued_native_keys_publish_bounded_batches_without_reordering_text() {
    use super::{Events, NativeInput};
    use crossterm::event::{Event, KeyEvent};
    use runyte::{
        app::{App, FrameGeometry},
        config::Config,
        workspace::WorkspaceHost,
    };
    let root = tempfile::tempdir().unwrap();
    let mut host =
        WorkspaceHost::new(App::new_in_project(Config::default(), None, root.path()).unwrap());
    host.defer_frontend_presentation(true);
    let painted = host.prepare_frame(FrameGeometry::default()).id;
    let (send, receive) = super::input_queue::channel(256);
    let mut events = Events::Native {
        attachment: std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0)),
        events: receive,
        batch: 0,
        presented: None,
        presentation_only: false,
        routing_serial: 0,
    };
    // Insert mode followed by 128 distinct ordered characters: three frames,
    // with the first two published despite additional input remaining queued.
    let text: String = (0..128).map(|i| char::from(b'a' + i % 26)).collect();
    for c in std::iter::once('i').chain(text.chars()) {
        send.try_send(NativeInput {
            attachment: 0,
            event: Event::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)),
            presented: Some(painted),
            presentation_only: false,
            routing_serial: 0,
        })
        .unwrap();
    }
    let mut pending = false;
    let mut publications = Vec::new();
    for n in 1..=129 {
        let input = runyte::tui::input::convert_event(events.next().await.unwrap().unwrap())
            .unwrap()
            .unwrap();
        assert_eq!(events.presented_frame(None), Some(painted));
        assert!(events.accepts_input(&host, &input, pending));
        host.execute_frontend_input(input, false).unwrap();
        if events.defer_frame(true, &mut pending) {
            assert!(pending);
        } else {
            publications.push(n);
            host.prepare_frame(FrameGeometry::default());
            pending = false;
        }
    }
    assert_eq!(publications, [64, 128, 129]);
    assert_eq!(host.app().active_buffer().to_string(), text);
    assert!(!pending);
}

#[tokio::test]
async fn native_batching_never_waits_for_acknowledgements_or_crosses_pointer_and_resize() {
    use super::{Events, NativeInput};
    use crossterm::event::{Event, KeyEvent, MouseButton, MouseEvent, MouseEventKind};
    let (send, receive) = super::input_queue::channel(8);
    let mut events = Events::Native {
        attachment: std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0)),
        events: receive,
        batch: 0,
        presented: None,
        presentation_only: false,
        routing_serial: 0,
    };
    let mut pending = false;
    assert!(
        !events.defer_frame(true, &mut pending),
        "a lone event paints immediately"
    );
    send.try_send(NativeInput {
        attachment: 0,
        event: Event::FocusGained,
        presented: None,
        presentation_only: true,
        routing_serial: 0,
    })
    .unwrap();
    assert!(
        !events.defer_frame(true, &mut pending),
        "a paint acknowledgement is not a queued edit"
    );
    events.next().await.unwrap().unwrap();
    assert!(events.is_presentation_acknowledgement());
    for barrier in [
        Event::Resize(100, 30),
        Event::FocusLost,
        Event::Mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 2,
            row: 3,
            modifiers: KeyModifiers::NONE,
        }),
    ] {
        send.try_send(NativeInput {
            attachment: 0,
            event: barrier.clone(),
            presented: None,
            presentation_only: false,
            routing_serial: 0,
        })
        .unwrap();
        send.try_send(NativeInput {
            attachment: 0,
            event: Event::Key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE)),
            presented: None,
            presentation_only: false,
            routing_serial: 0,
        })
        .unwrap();
        assert!(
            !events.defer_frame(true, &mut pending),
            "publish before the boundary"
        );
        assert_eq!(events.next().await.unwrap().unwrap(), barrier);
        assert!(
            !events.defer_frame(false, &mut pending),
            "publish after lifecycle/pointer handling"
        );
        events.next().await.unwrap().unwrap();
        assert!(!events.defer_frame(true, &mut pending));
    }
    assert!(!pending);
}

#[tokio::test]
async fn native_batch_pending_blocks_approval_even_before_frame_identity_changes() {
    use super::{Events, NativeInput};
    use crossterm::event::{Event, KeyEvent};
    use runyte::{
        app::{App, FrameGeometry, FsConfirmation, FsConfirmationOrigin},
        config::Config,
        fs_plan::{DesiredEntry, DirectorySnapshot, EntryKind, FsPlan},
        workspace::WorkspaceHost,
    };
    let root = tempfile::tempdir().unwrap();
    let mut host =
        WorkspaceHost::new(App::new_in_project(Config::default(), None, root.path()).unwrap());
    host.defer_frontend_presentation(true);
    let painted = host.prepare_frame(FrameGeometry::default()).id;
    let (send, receive) = super::input_queue::channel(8);
    let mut events = Events::Native {
        attachment: std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0)),
        events: receive,
        batch: 0,
        presented: None,
        presentation_only: false,
        routing_serial: 0,
    };
    send.try_send(NativeInput {
        attachment: 0,
        event: Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        presented: Some(painted),
        presentation_only: false,
        routing_serial: 0,
    })
    .unwrap();
    // Model a preceding input opening a confirmation, before preparing any new
    // frame. The captured id still equals the host id, so pending is essential.
    host.app_mut().fs_confirmation = Some(FsConfirmation {
        origin: FsConfirmationOrigin::Explorer {
            buffer: host.app().active().buffer,
        },
        plan: FsPlan::build(
            root.path().to_path_buf(),
            DirectorySnapshot::read(root.path()).unwrap(),
            vec![DesiredEntry::create("unseen.txt", EntryKind::File)],
        )
        .unwrap(),
        selected: 0,
    });
    let mut pending = false;
    assert!(events.defer_frame(true, &mut pending));
    let input = runyte::tui::input::convert_event(events.next().await.unwrap().unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(events.presented_frame(None), host.current_frame_id());
    assert!(
        events.accepts_input(&host, &input, false),
        "ids alone cannot detect unpublished state"
    );
    assert!(!events.accepts_input(&host, &input, pending));
    assert!(!root.path().join("unseen.txt").exists());
    assert!(host.app().fs_confirmation.is_some());
    assert!(!events.defer_frame(true, &mut pending));
}
