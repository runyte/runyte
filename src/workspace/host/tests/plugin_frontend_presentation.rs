// SPDX-License-Identifier: MPL-2.0

use super::*;
use crate::input::{InputEvent, KeyStroke};

fn geometry() -> crate::app::FrameGeometry {
    crate::ui::frame_geometry(ratatui::layout::Rect::new(0, 0, 80, 24))
}

fn input(key: &str) -> InputEvent {
    InputEvent::Key(KeyStroke::parse(key).unwrap())
}

#[test]
fn asynchronous_lsp_permission_waits_for_painted_approval() {
    let (root, mut host) = host();
    host.defer_frontend_presentation(true);
    let before = host.prepare_frame(geometry()).id;
    host.app.configure_lsp_trust(Some(root.join("trust")));
    assert!(host.app.input_requires_presented_approval(&input("Enter")));
    assert!(!host.accepts_frontend_input(&input("Enter"), Some(before), true));
    assert!(!host.accepts_frontend_input(&input("Enter"), None, false));
    assert!(host.accepts_frontend_input(&input("Down"), Some(before), true));
    host.execute_frontend_input(input("Down"), false).unwrap();
    let approval = host.prepare_frame(geometry()).id;
    assert!(!host.accepts_frontend_input(&input("Enter"), Some(before), false));
    assert!(host.accepts_frontend_input(&input("Enter"), Some(approval), false));
    host.execute_frontend_input(input("Enter"), false).unwrap();
    assert_eq!(host.app.status, "LSP allowed for this workspace");
}

#[test]
fn asynchronous_form_keeps_queued_typing_but_requires_painted_submission() {
    use crate::plugin::interaction::{Field, Value};
    let (_root, mut host) = host();
    let mut output = setup(&mut host, 0, &["interaction"]);
    next(&mut output);
    interaction::begin(
        &mut host,
        &mut output,
        1,
        vec![Field::text("name", "Name".into())],
    );
    host.defer_frontend_presentation(true);
    let painted = host.prepare_frame(geometry()).id;
    for key in ["a", "b", "Backspace", "c"] {
        let event = input(key);
        assert!(host.accepts_frontend_input(&event, Some(painted), true));
        host.execute_frontend_input(event, false).unwrap();
        host.prepare_frame(geometry());
    }
    assert_eq!(
        host.app.plugins.input.as_ref().unwrap().values[0],
        Value::Text("ac".into())
    );
    assert!(!host.accepts_frontend_input(&input("Enter"), Some(painted), false));
    let current = host.current_frame_id();
    assert!(host.accepts_frontend_input(&input("Enter"), current, false));
    assert!(!host.accepts_frontend_input(&input("Enter"), current, true));
    host.execute_frontend_input(input("Enter"), false).unwrap();
    assert!(host.app.plugins.input.is_none());
    assert_eq!(
        host.app.plugins.input_finished[0].2.values["name"],
        Value::Text("ac".into())
    );
}

#[tokio::test]
async fn asynchronous_plugin_actions_use_painted_revision_from_every_entry_point() {
    for route in ["binding", "menu", "command"] {
        let (_root, mut host) = host();
        let mut receiver = view_setup_with_binding(&mut host, Some("F12"));
        host.defer_frontend_presentation(true);
        model_request(
            &mut host,
            0,
            1,
            api::Request::ViewCreate {
                model: model(&[("one", "Original")]),
            },
        )
        .await;
        let (view, revision) = view_result(&mut receiver);
        show_view(&mut host, &mut receiver, &view, 2);
        let painted = host.prepare_frame(geometry()).id;
        assert!(host.app.plugins.presented_views.is_empty());
        host.acknowledge_frontend_input_frame(Some(painted));
        assert!(!host.app.plugins.presented_views.is_empty());
        if route == "menu" {
            host.execute_frontend_input(input("Tab"), false).unwrap();
        } else if route == "command" {
            for character in ":plugin.tasks.toggle".chars() {
                host.execute_frontend_input(
                    InputEvent::Key(KeyStroke::new(
                        crate::input::KeyCode::Char(character),
                        crate::input::Modifiers::NONE,
                    )),
                    false,
                )
                .unwrap();
            }
        }
        let painted = host.prepare_frame(geometry()).id;
        model_request(
            &mut host,
            0,
            3,
            api::Request::ViewPublish {
                expected_query_revision: None,
                view: view.clone(),
                expected_revision: revision,
                model: model(&[("one", "Changed without changing row identity")]),
            },
        )
        .await;
        view_result(&mut receiver);
        let unseen = host.prepare_frame(geometry()).id;
        host.acknowledge_frontend_input_frame(Some(painted));
        let key = if route == "binding" { "F12" } else { "Enter" };
        assert!(host.accepts_frontend_input(&input(key), Some(painted), false));
        host.execute_frontend_input(input(key), false).unwrap();
        assert!(
            host.app.status.contains("wait for refresh"),
            "{route}: {}",
            host.app.status
        );
        while let Ok(message) = receiver.try_recv() {
            assert!(!matches!(
                message,
                HostMessage::Application(api::HostMessage::Request { .. })
            ));
        }
        host.acknowledge_frontend_input_frame(Some(unseen));
        assert!(matches!(
            invoke(&mut host, "plugin.tasks.toggle"),
            CommandOutcome::AsynchronousRequest(_)
        ));
        assert!(
            matches!(next(&mut receiver), api::HostMessage::Request { params, .. } if params.model_revision.as_deref() == Some("m:2"))
        );
    }
}

#[tokio::test]
async fn asynchronous_plugin_presentation_history_is_bounded_and_fails_closed() {
    let (_root, mut host) = host();
    let mut receiver = view_setup(&mut host);
    host.defer_frontend_presentation(true);
    model_request(
        &mut host,
        0,
        1,
        api::Request::ViewCreate {
            model: model(&[("one", "Original")]),
        },
    )
    .await;
    let (view, _) = view_result(&mut receiver);
    show_view(&mut host, &mut receiver, &view, 2);
    let watch = (0, view, host.app.active_pane);
    host.app.plugins.viewport_watches.insert(watch.clone());
    let expired = host.prepare_frame(geometry()).id;
    for _ in 0..32 {
        host.prepare_frame(geometry());
    }
    assert_eq!(host.plugin_presentations.len(), 32);
    let before_input = host.app.plugins.foreground_generation;
    host.acknowledge_frontend_paint(host.current_frame_id());
    assert!(!host.app.plugins.presented_views.is_empty());
    assert_eq!(host.app.plugins.foreground_generation, before_input);
    assert!(
        matches!(&host.app.plugins.viewport_cache[&watch], crate::plugin::observation::Snapshot::Viewport { model_revision: Some(revision), visible: true, .. } if revision == "m:1")
    );
    for frame in [None, Some(expired)] {
        host.acknowledge_frontend_input_frame(frame);
        assert!(host.app.plugins.presented_views.is_empty());
        assert!(matches!(
            invoke(&mut host, "plugin.tasks.toggle"),
            CommandOutcome::UserError(_)
        ));
    }
}
