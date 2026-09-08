// SPDX-License-Identifier: MPL-2.0

use super::plugin_rich_views::{commands, fixture, model};
use super::*;
use crate::plugin::{
    self, application as api,
    observation::{Action, Delivery, Snapshot, Source},
    view,
};

fn application(
    value: view::Model,
) -> (App, usize, tokio::sync::mpsc::Receiver<plugin::HostMessage>) {
    let (mut app, buffer, projection) = fixture(value.clone());
    app.note_plugin_frontend(true);
    let receiver = commands(&mut app, buffer, value, projection);
    (app, buffer, receiver)
}
fn query(app: &mut App, revision: u64, pending: bool) {
    app.plugins
        .instances
        .get_mut(&0)
        .unwrap()
        .application
        .views
        .get_mut("v:1")
        .unwrap()
        .query = Some(view::QueryState {
        revision,
        text: "猫".into(),
        pending,
    });
}
fn frame(app: &mut App, width: u16, height: u16) -> PreparedView {
    app.prepare_view(FrameGeometry {
        screen: Rect {
            width,
            height,
            ..Rect::default()
        },
        editor: Rect {
            width,
            height,
            ..Rect::default()
        },
        status: Rect::default(),
        message: Rect::default(),
    })
}
fn watch(app: &mut App) -> usize {
    let pane = app.active_pane;
    app.set_plugin_viewport_watches([(0, "v:1".into(), pane)].into());
    pane
}
fn observe_actions(app: &mut App) -> Source {
    let source = Source::ViewActions { view: "v:1".into() };
    app.plugins
        .instances
        .get_mut(&0)
        .unwrap()
        .application
        .observations
        .subscribe(
            "s:1".into(),
            vec![source.clone()],
            vec![(
                source.clone(),
                Snapshot::ViewActions {
                    accepted: "a:0".into(),
                },
            )],
        )
        .unwrap();
    source
}

#[test]
fn pending_query_refuses_primary_and_passes_empty_rows_to_nonprimary_callbacks() {
    let (mut app, _, mut receiver) = application(model(&["a", "b"]));
    query(&mut app, 1, true);
    assert!(matches!(
        app.invoke_plugin_arguments(1, "").unwrap(),
        CommandOutcome::UserError(_)
    ));
    assert!(receiver.try_recv().is_err());
    assert!(app.open_plugin_actions());
    assert_eq!(app.list_actions.len(), 2);
    assert!(
        app.list_actions
            .iter()
            .all(|action| !matches!(action, ListAction::PluginCommand { command: 1, .. }))
    );
    assert!(matches!(
        app.invoke_plugin_arguments(2, "").unwrap(),
        CommandOutcome::AsynchronousRequest(_)
    ));
    match receiver.try_recv().unwrap() {
        plugin::HostMessage::Application(api::HostMessage::Request { params, .. }) => {
            assert!(params.rows.is_empty());
            assert_eq!(params.query_revision.as_deref(), Some("qv:1"));
            assert_eq!(params.command, "refresh");
        }
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn menu_entry_captured_before_query_change_is_refused_even_after_query_completes() {
    let (mut app, _, mut receiver) = application(model(&["a"]));
    assert!(app.open_plugin_actions());
    app.list.as_mut().unwrap().selected = 1; // Nonprimary refresh also binds the seen query.
    query(&mut app, 1, false);
    app.handle_list_key(KeyStroke::parse("Enter").unwrap())
        .unwrap();
    assert!(receiver.try_recv().is_err());
    assert!(app.status.contains("reopen the actions"));
}

#[test]
fn query_completion_needs_new_model_presentation_before_primary_dispatch() {
    let (mut app, _, mut receiver) = application(model(&["a"]));
    query(&mut app, 3, false);
    app.plugins
        .instances
        .get_mut(&0)
        .unwrap()
        .application
        .views
        .get_mut("v:1")
        .unwrap()
        .revision = 2;
    assert!(matches!(
        app.invoke_plugin_arguments(1, "").unwrap(),
        CommandOutcome::UserError(_)
    ));
    assert!(receiver.try_recv().is_err());
    frame(&mut app, 80, 20);
    app.invoke_plugin_arguments(1, "").unwrap();
    match receiver.try_recv().unwrap() {
        plugin::HostMessage::Application(api::HostMessage::Request { params, .. }) => {
            assert_eq!(params.query_revision.as_deref(), Some("qv:3"));
            assert_eq!(params.model_revision.as_deref(), Some("m:2"));
            assert_eq!(params.rows, ["a"]);
        }
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn accepted_action_observation_correlates_only_fully_admitted_callback() {
    let (mut app, _, mut receiver) = application(model(&["a"]));
    observe_actions(&mut app);
    app.invoke_plugin_arguments(1, "").unwrap();
    let request = match receiver.try_recv().unwrap() {
        plugin::HostMessage::Application(api::HostMessage::Request { id, .. }) => id,
        other => panic!("unexpected {other:?}"),
    };
    assert!(matches!(
        receiver.try_recv().unwrap(),
        plugin::HostMessage::Deadline { .. }
    ));
    let state = &mut app.plugins.instances.get_mut(&0).unwrap().application;
    assert_eq!(state.views["v:1"].accepted_actions, 1);
    match state.observations.peek_ready().unwrap() {
        Delivery::Action(event) => {
            assert_eq!(event.action.id, "a:1");
            assert_eq!(event.action.request, request);
            assert_eq!(event.action.command, "enter");
            assert_eq!(event.action.selected_count, 1);
            assert_eq!(event.action.query_revision, None);
        }
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn partial_callback_admission_stops_owner_and_never_records_an_accepted_action() {
    let (mut app, _, _) = application(model(&["a"]));
    observe_actions(&mut app);
    let (sender, mut receiver) = tokio::sync::mpsc::channel(1);
    app.plugins.instances.get_mut(&0).unwrap().sender = plugin::Sender::new(sender);
    assert!(matches!(
        app.invoke_plugin_arguments(1, "").unwrap(),
        CommandOutcome::UserError(_)
    ));
    assert!(matches!(
        receiver.try_recv().unwrap(),
        plugin::HostMessage::Application(api::HostMessage::Request { .. })
    ));
    assert!(app.plugins.cancellations.contains(&0));
    let state = &app.plugins.instances[&0].application;
    assert!(state.requests.is_empty());
    assert_eq!(state.views["v:1"].accepted_actions, 0);
    assert!(state.observations.peek_ready().is_none());
    assert!(matches!(
        app.invoke_plugin_arguments(2, "").unwrap(),
        CommandOutcome::UserError(_)
    ));
    assert!(receiver.try_recv().is_err());
}

#[test]
fn full_reliable_action_queue_refuses_before_callback_admission() {
    let (mut app, _, mut receiver) = application(model(&["a"]));
    let source = observe_actions(&mut app);
    for index in 0..plugin::observation::MAX_RELIABLE {
        app.plugins
            .instances
            .get_mut(&0)
            .unwrap()
            .application
            .observations
            .record_action(
                &source,
                Action {
                    id: format!("a:{}", index + 1),
                    request: format!("h:{}", index + 1),
                    command: "enter".into(),
                    pane: "p:1".into(),
                    model_revision: "m:1".into(),
                    query_revision: None,
                    selection_revision: "q:0".into(),
                    selected_count: 1,
                },
            )
            .unwrap();
    }
    assert!(matches!(
        app.invoke_plugin_arguments(1, "").unwrap(),
        CommandOutcome::UserError(_)
    ));
    assert!(receiver.try_recv().is_err());
    assert_eq!(
        app.plugins.instances[&0].application.views["v:1"].accepted_actions,
        0
    );
}

#[test]
fn viewport_cache_is_opt_in_and_unchanged_frames_do_not_dirty_presentation() {
    let (mut app, _, _receiver) = application(model(&["a", "b"]));
    let prepared = frame(&mut app, 80, 20);
    assert!(app.plugins.viewport_cache.is_empty());
    let pane = watch(&mut app);
    app.capture_plugin_viewports(&prepared);
    assert_eq!(
        app.plugin_viewport(0, "v:1", pane),
        Some(&Snapshot::Viewport {
            model_revision: Some("m:1".into()),
            visible: true,
            top: Some("a".into()),
            bottom: Some("b".into()),
        })
    );
    let held = app.plugins.viewport_cache.clone();
    app.plugins.presentation_dirty = false;
    frame(&mut app, 80, 20);
    assert_eq!(app.plugins.viewport_cache, held);
    assert!(!app.plugins.presentation_dirty);
    app.set_plugin_viewport_watches(Default::default());
    frame(&mut app, 80, 20);
    assert!(app.plugins.viewport_cache.is_empty());
}

#[test]
fn viewport_uses_final_wrapped_rows_and_excludes_nonrow_headers_and_hidden_panes() {
    let mut value = model(&["a", "b"]);
    value.rows[0].text = "long 猫 ".repeat(20);
    value.status = Some(view::Block {
        text: "Status".into(),
        role: view::Role::Muted,
    });
    let (mut app, buffer, _receiver) = application(value);
    app.config.editor.soft_wrap = true;
    let pane = watch(&mut app);
    let prepared = frame(&mut app, 15, 6);
    let p = prepared.pane(pane).unwrap();
    assert!(p.rows.iter().any(|row| row.continuation));
    let ids: Vec<_> = p
        .rows
        .iter()
        .filter_map(|row| row.document_row)
        .filter_map(|line| {
            app.plugins.instances[&0].application.views["v:1"]
                .projection
                .line_rows[line]
        })
        .map(|index| {
            app.plugins.instances[&0].application.views["v:1"]
                .projection
                .rows[index]
                .id
                .clone()
        })
        .collect();
    assert_eq!(
        app.plugin_viewport(0, "v:1", pane),
        Some(&Snapshot::Viewport {
            model_revision: Some("m:1".into()),
            visible: true,
            top: ids.first().cloned(),
            bottom: ids.last().cloned(),
        })
    );
    app.switch_buffer(0);
    app.capture_plugin_viewports(&prepared);
    assert_eq!(
        app.plugin_viewport(0, "v:1", pane),
        Some(&Snapshot::Viewport {
            model_revision: None,
            visible: false,
            top: None,
            bottom: None,
        })
    );
    app.switch_buffer(buffer);
    app.plugins
        .instances
        .get_mut(&0)
        .unwrap()
        .application
        .views
        .get_mut("v:1")
        .unwrap()
        .revision = 2;
    app.capture_plugin_viewports(&prepared);
    assert!(matches!(
        app.plugin_viewport(0, "v:1", pane),
        Some(Snapshot::Viewport {
            visible: false,
            model_revision: None,
            ..
        })
    ));
}

#[test]
fn viewport_split_and_maximized_panes_keep_independent_endpoints() {
    let (mut app, _, _receiver) = application(model(&["a", "b", "c", "d", "e", "f"]));
    let first = app.active_pane;
    app.split(Axis::Horizontal, None).unwrap();
    let second = app.active_pane;
    app.active_mut().scroll_row = 3;
    app.active_mut().preserve_scroll = true;
    app.set_plugin_viewport_watches([(0, "v:1".into(), first), (0, "v:1".into(), second)].into());
    frame(&mut app, 80, 5);
    assert!(
        matches!(app.plugin_viewport(0,"v:1",first),Some(Snapshot::Viewport{top:Some(top),..}) if top=="a")
    );
    assert!(
        matches!(app.plugin_viewport(0,"v:1",second),Some(Snapshot::Viewport{top:Some(top),..}) if top=="d")
    );
    app.maximized = Some(MaximizedPane {
        pane: second,
        view: MaximizedView::Fullscreen,
    });
    frame(&mut app, 80, 5);
    assert!(matches!(
        app.plugin_viewport(0, "v:1", first),
        Some(Snapshot::Viewport { visible: false, .. })
    ));
    assert!(matches!(
        app.plugin_viewport(0, "v:1", second),
        Some(Snapshot::Viewport { visible: true, .. })
    ));
}

#[test]
fn viewport_getter_invalidates_navigation_terminal_and_attachment_without_a_frame() {
    let (mut app, buffer, _receiver) = application(model(&["a"]));
    let pane = watch(&mut app);
    let prepared = frame(&mut app, 80, 20);
    app.switch_buffer(0);
    assert!(matches!(
        app.plugin_viewport(0, "v:1", pane),
        Some(Snapshot::Viewport { visible: false, .. })
    ));
    app.switch_buffer(buffer);
    app.active_mut().terminal = Some(TerminalId::from_raw(999));
    assert!(matches!(
        app.plugin_viewport(0, "v:1", pane),
        Some(Snapshot::Viewport { visible: false, .. })
    ));
    app.active_mut().terminal = None;
    app.note_plugin_frontend(false);
    assert!(app.plugin_viewport(0, "v:1", pane).is_none());
    app.note_plugin_frontend(true);
    assert!(app.plugin_viewport(0, "v:1", pane).is_none());
    app.capture_plugin_viewports(&prepared);
    assert!(matches!(
        app.plugin_viewport(0, "v:1", pane),
        Some(Snapshot::Viewport { visible: false, .. })
    ));
    frame(&mut app, 80, 20);
    assert!(matches!(
        app.plugin_viewport(0, "v:1", pane),
        Some(Snapshot::Viewport { visible: true, .. })
    ));
}

#[test]
fn existing_viewport_keeps_its_presented_model_until_next_frame() {
    let (mut app, _, _receiver) = application(model(&["a"]));
    let pane = watch(&mut app);
    let prepared = frame(&mut app, 80, 20);
    let old = app.plugin_viewport(0, "v:1", pane).cloned();
    app.plugins
        .instances
        .get_mut(&0)
        .unwrap()
        .application
        .views
        .get_mut("v:1")
        .unwrap()
        .revision = 2;
    app.capture_plugin_viewports(&prepared);
    assert_eq!(app.plugin_viewport(0, "v:1", pane), old.as_ref());
    frame(&mut app, 80, 20);
    assert!(
        matches!(app.plugin_viewport(0,"v:1",pane),Some(Snapshot::Viewport{model_revision:Some(revision),..}) if revision=="m:2")
    );
}

#[test]
fn oversized_escaped_row_selection_is_refused_without_stopping_owner_or_dispatch() {
    let mut value = model(&[]);
    value.rows = (0..view::MAX_ROWS)
        .map(|index| view::Row {
            id: format!("{}{:04}", "\"".repeat(60), index),
            text: "x".into(),
            ..Default::default()
        })
        .collect();
    let (mut app, buffer, mut receiver) = application(value);
    observe_actions(&mut app);
    let chars = app.buffers[buffer].len_chars();
    app.active_mut()
        .replace_selection(Selection::single(Range::new(0, chars)));
    assert!(matches!(
        app.invoke_plugin_arguments(1, "").unwrap(),
        CommandOutcome::UserError(_)
    ));
    assert!(app.status.contains("select fewer rows"));
    assert!(receiver.try_recv().is_err());
    assert!(!app.plugins.cancellations.contains(&0));
    let state = &app.plugins.instances[&0].application;
    assert!(state.requests.is_empty());
    assert_eq!(state.views["v:1"].accepted_actions, 0);
    assert!(state.observations.peek_ready().is_none());
}
