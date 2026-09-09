// SPDX-License-Identifier: MPL-2.0

use super::*;
use crate::buffer::{PluginProjectionSource, PreparedPluginProjection};
use crate::plugin::{self, application as api, view};
use std::sync::{Arc, atomic::AtomicBool};

pub(super) fn model(ids: &[&str]) -> view::Model {
    serde_json::from_value(serde_json::json!({
        "title":"Rich list", "purpose":"list",
        "rows": ids.iter().map(|id| serde_json::json!({"id":id,"text":format!("{id} 猫"),"role":"ordinary" })).collect::<Vec<_>>()
    })).unwrap()
}

fn prepare(
    model: view::Model,
    source: PluginProjectionSource,
) -> (
    view::PreparedModel,
    PreparedPluginProjection,
    Vec<crate::syntax::Span>,
) {
    let mut model = view::PreparedModel::build(model, &AtomicBool::new(false)).unwrap();
    let body = std::mem::take(&mut Arc::get_mut(&mut model.projection).unwrap().text);
    let text = source.prepare(body);
    let spans = model.projection.spans.clone();
    (model, text, spans)
}

pub(super) fn fixture(model: view::Model) -> (App, usize, Arc<view::Projection>) {
    let mut app = App::new(Config::default(), None).unwrap();
    let (model, text, spans) = prepare(model, PluginProjectionSource::empty());
    let buffer = app.create_prepared_plugin_view(0, "v:1", &model, text, spans);
    app.switch_buffer(buffer);
    (app, buffer, model.projection)
}

pub(super) fn publish(
    app: &mut App,
    buffer: usize,
    old: &view::Projection,
    model: view::Model,
) -> Arc<view::Projection> {
    let (model, text, spans) = prepare(model, app.buffers[buffer].plugin_projection_source());
    let remap = model.projection.remap_from(old);
    assert!(app.publish_prepared_plugin_view(buffer, old, &model, text, spans, &remap));
    model.projection
}

#[test]
fn rich_projection_installs_unicode_columns_blocks_and_highlights_as_read_only_text() {
    let model = serde_json::from_value(serde_json::json!({
        "title":"Tasks", "purpose":"dashboard",
        "columns":[{"id":"name","label":"Name"},{"id":"state","label":"State"}],
        "rows":[{"id":"猫","text":"","role":"ordinary","cells":[{"text":"猫🐈","role":"heading"},{"text":"Ready","role":"muted"}]}],
        "status":{"text":"Connected","role":"muted"},
        "detail":{"text":"first\n猫 second","role":"ordinary"},"preview":{"text":"α\nβ","role":"ordinary"}
    })).unwrap();
    let (app, buffer, projection) = fixture(model);
    let text = app.buffers[buffer].text().to_string();
    assert!(text.contains("猫🐈"));
    assert!(text.contains("first\n猫 second"));
    assert!(text.contains("α\nβ"));
    let row = &projection.rows[0];
    assert_eq!(app.buffers[buffer].offset_to_row(row.from), row.line);
    assert_eq!(projection.line_rows[row.line], Some(0));
    assert!(projection.line_rows[..row.line].iter().all(Option::is_none));
    assert!(
        projection.line_rows[row.line + 1..]
            .iter()
            .all(Option::is_none)
    );
    assert!(!app.generated_highlights[&buffer].is_empty());
    assert!(app.buffers[buffer].is_read_only());
    assert!(!app.buffers[buffer].dirty);
}

#[test]
fn rich_projection_reorder_preserves_selection_direction_and_commit_time_scroll() {
    for reverse in [false, true] {
        let (mut app, buffer, old) = fixture(model(&["a", "b", "c"]));
        let (new, text, spans) = prepare(
            model(&["c", "b", "a"]),
            app.buffers[buffer].plugin_projection_source(),
        );
        // This input happens after worker capture and must remain authoritative.
        let start = old.rows[0].from + 1;
        let end = old.rows[2].from + 2;
        app.active_mut()
            .replace_selection(Selection::single(if reverse {
                Range::new(end, start)
            } else {
                Range::new(start, end)
            }));
        app.active_mut().scroll_row = old.rows[2].line;
        let remap = new.projection.remap_from(&old);
        assert!(app.publish_prepared_plugin_view(buffer, &old, &new, text, spans, &remap));
        let selected = app.active().selection.primary();
        assert_eq!(selected.anchor > selected.head, reverse);
        assert_eq!(selected.from(), new.projection.rows[0].from + 2);
        assert_eq!(selected.to(), new.projection.rows[2].from + 1);
        assert_eq!(app.active().scroll_row, new.projection.rows[0].line);
        assert!(app.active().preserve_scroll);
    }
}

#[test]
fn rich_projection_deleted_rows_choose_old_order_neighbor_then_position_and_empty() {
    let (mut app, buffer, old) = fixture(model(&["a", "b", "c"]));
    app.active_mut()
        .replace_selection(Selection::point(old.rows[1].from));
    app.active_mut().scroll_row = old.rows[1].line;
    let new = publish(&mut app, buffer, &old, model(&["c", "a"]));
    // a and c were equally close to b; a wins even after the new order reverses.
    assert_eq!(app.active().selection.primary().head, new.rows[1].from);
    assert_eq!(app.active().scroll_row, new.rows[1].line);
    let replaced = publish(&mut app, buffer, &new, model(&["x", "y", "z"]));
    assert_eq!(app.active().selection.primary().head, replaced.rows[1].from);
    publish(&mut app, buffer, &replaced, model(&[]));
    assert_eq!(app.active().selection.primary().head, 0);
    assert_eq!(app.active().scroll_row, 0);
}

#[test]
fn rich_projection_stale_preparation_cannot_replace_newer_text_or_selection() {
    let (mut app, buffer, old) = fixture(model(&["a"]));
    let (stale, text, spans) = prepare(
        model(&["stale"]),
        app.buffers[buffer].plugin_projection_source(),
    );
    let current = publish(&mut app, buffer, &old, model(&["current"]));
    app.active_mut()
        .replace_selection(Selection::point(current.rows[0].from + 2));
    let revision = app.buffers[buffer].revision();
    let remap = stale.projection.remap_from(&old);
    assert!(!app.publish_prepared_plugin_view(buffer, &old, &stale, text, spans, &remap));
    assert_eq!(app.buffers[buffer].revision(), revision);
    assert_eq!(app.buffers[buffer].text().to_string(), "current 猫\n");
    assert_eq!(app.active().selection.primary().head, 2);
}

#[test]
fn rich_projection_hidden_update_preserves_active_pane_and_visible_selection() {
    let (mut app, buffer, old) = fixture(model(&["a"]));
    app.switch_buffer(0);
    let pane = app.active_pane;
    let selection = app.active().selection.clone();
    publish(&mut app, buffer, &old, model(&["b"]));
    assert_eq!(app.active_pane, pane);
    assert_eq!(app.active().buffer, 0);
    assert_eq!(app.active().selection, selection);
}

pub(super) fn commands(
    app: &mut App,
    buffer: usize,
    model: view::Model,
    projection: Arc<view::Projection>,
) -> tokio::sync::mpsc::Receiver<plugin::HostMessage> {
    use crate::app::plugin_workflows::{Instance, RuntimeCommand};
    let (sender, receiver) = tokio::sync::mpsc::channel(32);
    let mut application = api::Instance::default();
    application.primary_commands.insert("enter".into());
    application.views.insert(
        "v:1".into(),
        view::View {
            buffer,
            model: Arc::new(model),
            encoded: "{}".into(),
            projection,
            charge: 0,
            revision: 1,
            published: None,
            query: None,
            accepted_actions: 0,
        },
    );
    app.plugins.instances.insert(
        0,
        Instance {
            config: plugin::PluginConfig {
                settings: Default::default(),
                id: "test".into(),
                api: api::Api::Epoch2,
                capabilities: vec!["views".into()],
                enabled: true,
                executable: "/nonexistent/plugin".into(),
                args: vec![],
                bindings: Default::default(),
            },
            sender: plugin::Sender::new(sender),
            registered: true,
            application,
            pending: None,
            issued: Default::default(),
            subscriptions: Default::default(),
            sequence: 0,
        },
    );
    for (id, name) in [(1, "enter"), (2, "refresh"), (3, "hidden")] {
        app.plugins.commands.insert(
            id,
            RuntimeCommand {
                arguments: vec![],
                id,
                plugin: 0,
                name: format!("plugin.test.{name}"),
                usage: String::new(),
                local: name.into(),
                description: name.into(),
                binding: None,
                context: api::CommandContext::View,
            },
        );
    }
    app.plugins
        .presented_views
        .insert(app.active_pane, (buffer, 1));
    receiver
}

#[test]
fn rich_projection_headers_reject_primary_but_allow_refresh_and_gate_all_actions() {
    let mut value = model(&["a"]);
    value.status = Some(view::Block {
        text: "Status".into(),
        role: view::Role::Muted,
    });
    value.actions = vec!["enter".into(), "refresh".into()];
    let (mut app, buffer, projection) = fixture(value.clone());
    let row_offset = projection.rows[0].from;
    let mut receiver = commands(&mut app, buffer, value, projection);
    app.active_mut().replace_selection(Selection::point(0));
    assert!(matches!(
        app.invoke_plugin_arguments(1, "").unwrap(),
        CommandOutcome::UserError(_)
    ));
    assert!(receiver.try_recv().is_err());
    assert!(matches!(
        app.invoke_plugin_arguments(2, "").unwrap(),
        CommandOutcome::AsynchronousRequest(_)
    ));
    match receiver.try_recv().unwrap() {
        plugin::HostMessage::Application(api::HostMessage::Request { params, .. }) => {
            assert!(params.rows.is_empty())
        }
        other => panic!("unexpected {other:?}"),
    }
    receiver.try_recv().unwrap(); // Deadline.
    app.active_mut()
        .replace_selection(Selection::point(row_offset));
    app.invoke_plugin_arguments(1, "").unwrap();
    match receiver.try_recv().unwrap() {
        plugin::HostMessage::Application(api::HostMessage::Request { params, .. }) => {
            assert_eq!(params.rows, ["a"])
        }
        other => panic!("unexpected {other:?}"),
    }
    receiver.try_recv().unwrap();
    assert!(matches!(
        app.invoke_plugin_arguments(3, "").unwrap(),
        CommandOutcome::UserError(_)
    ));
    assert!(receiver.try_recv().is_err());
    assert!(app.open_plugin_actions());
    assert_eq!(app.list_actions.len(), 2);
}

#[test]
fn rich_projection_empty_view_retains_nonprimary_legacy_actions() {
    let value = model(&[]);
    let (mut app, buffer, projection) = fixture(value.clone());
    let mut receiver = commands(&mut app, buffer, value, projection);
    assert!(matches!(
        app.invoke_plugin_arguments(1, "").unwrap(),
        CommandOutcome::UserError(_)
    ));
    assert!(matches!(
        app.invoke_plugin_arguments(3, "").unwrap(),
        CommandOutcome::AsynchronousRequest(_)
    ));
    match receiver.try_recv().unwrap() {
        plugin::HostMessage::Application(api::HostMessage::Request { params, .. }) => {
            assert!(params.rows.is_empty())
        }
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn rich_projection_nonrow_selection_stays_positional_across_publication_and_return() {
    let mut value = model(&["a"]);
    value.status = Some(view::Block {
        text: "Connected".into(),
        role: view::Role::Muted,
    });
    let (mut app, buffer, old) = fixture(value.clone());
    app.active_mut().replace_selection(Selection::point(3));
    let new = publish(&mut app, buffer, &old, value.clone());
    assert_eq!(app.active().selection.primary().head, 3);
    let _receiver = commands(&mut app, buffer, value, new.clone());
    app.switch_buffer(0);
    app.present_plugin_view(buffer);
    assert_eq!(app.active().selection.primary().head, 3);
    app.active_mut().replace_selection(Selection::point(2));
    app.present_plugin_view(buffer);
    assert_eq!(app.active().selection.primary().head, 2);
}

#[test]
fn rich_projection_multiple_ranges_keep_primary_identity_and_clamp_shorter_columns() {
    let (mut app, buffer, old) = fixture(model(&["alpha", "beta", "gamma"]));
    app.active_mut().replace_selection(Selection::new(
        vec![
            Range::new(old.rows[0].from + 4, old.rows[0].from + 1),
            Range::new(old.rows[2].from + 1, old.rows[2].from + 4),
        ],
        1,
    ));
    let mut next = model(&["gamma", "beta", "alpha"]);
    next.rows[0].text = "猫".into();
    let new = publish(&mut app, buffer, &old, next);
    assert_eq!(app.active().selection.ranges().len(), 2);
    let primary = app.active().selection.primary();
    assert_eq!(primary.anchor, new.rows[0].from + 1);
    assert_eq!(primary.head, new.rows[0].from + 1);
    let other = app
        .active()
        .selection
        .ranges()
        .iter()
        .find(|range| range.anchor != primary.anchor)
        .unwrap();
    assert!(other.anchor > other.head);
    assert_eq!(other.head, new.rows[2].from + 1);
}

#[test]
fn rich_projection_return_restores_saved_selection_and_scroll_after_hidden_reorder() {
    let value = model(&["a", "b", "c"]);
    let (mut app, buffer, old) = fixture(value.clone());
    let _receiver = commands(&mut app, buffer, value, old.clone());
    app.present_plugin_view(buffer);
    app.active_mut()
        .replace_selection(Selection::single(Range::new(
            old.rows[1].from + 2,
            old.rows[1].from,
        )));
    app.active_mut().scroll_row = old.rows[1].line;
    app.active_mut().scroll_col = 2;
    app.switch_buffer(0);
    let selection = app.active().selection.clone();
    let new = publish(&mut app, buffer, &old, model(&["b", "c", "a"]));
    assert_eq!(app.active().buffer, 0);
    assert_eq!(app.active().selection, selection);
    app.present_plugin_view(buffer);
    assert_eq!(
        app.active().selection.primary(),
        Range::new(new.rows[0].from + 2, new.rows[0].from)
    );
    assert_eq!(app.active().scroll_row, new.rows[0].line);
    assert_eq!(app.active().scroll_col, 2);
    app.switch_buffer(0);
    app.present_plugin_view(buffer);
    assert_eq!(app.active().selection.primary().head, new.rows[0].from);
    app.host_close_buffer(buffer, false).unwrap();
    assert!(
        app.panes
            .values()
            .all(|pane| !pane.plugin_view_positions.contains_key(&buffer))
    );
}

#[test]
fn rich_projection_native_switch_and_jump_restore_stable_rows_after_hidden_update() {
    for jump in [false, true] {
        let value = model(&["a", "b", "c"]);
        let (mut app, buffer, old) = fixture(value.clone());
        let _receiver = commands(&mut app, buffer, value, old.clone());
        app.present_plugin_view(buffer);
        app.active_mut()
            .replace_selection(Selection::point(old.rows[1].from + 1));
        app.active_mut().scroll_row = old.rows[1].line;
        app.switch_buffer(0);
        let new = publish(&mut app, buffer, &old, model(&["b", "a", "c"]));
        if jump {
            app.jump_in(true, true);
        } else {
            app.switch_buffer(buffer);
        }
        assert_eq!(app.active().buffer, buffer);
        assert_eq!(app.active().selection.primary().head, new.rows[0].from + 1);
        if !jump {
            assert_eq!(app.active().scroll_row, new.rows[0].line);
        }
    }
}

#[test]
fn rich_projection_saved_positions_remain_independent_between_panes() {
    let value = model(&["a", "b", "c"]);
    let (mut app, buffer, old) = fixture(value.clone());
    let _receiver = commands(&mut app, buffer, value, old.clone());
    app.present_plugin_view(buffer);
    let first = app.active_pane;
    app.active_mut()
        .replace_selection(Selection::point(old.rows[0].from + 1));
    app.split(Axis::Vertical, None).unwrap();
    let second = app.active_pane;
    app.active_mut()
        .replace_selection(Selection::point(old.rows[2].from + 2));
    app.switch_buffer(0);
    app.activate_pane(first);
    app.switch_buffer(0);
    let new = publish(&mut app, buffer, &old, model(&["c", "b", "a"]));
    app.present_plugin_view(buffer);
    assert_eq!(app.active().selection.primary().head, new.rows[2].from + 1);
    app.activate_pane(second);
    app.present_plugin_view(buffer);
    assert_eq!(app.active().selection.primary().head, new.rows[0].from + 2);
}

#[test]
fn rich_projection_first_native_presentation_starts_at_first_actionable_row() {
    let mut app = App::new(Config::default(), None).unwrap();
    let mut value = model(&["a"]);
    value.status = Some(view::Block {
        text: "Connected".into(),
        role: view::Role::Muted,
    });
    let (prepared, text, spans) = prepare(value.clone(), PluginProjectionSource::empty());
    let buffer = app.create_prepared_plugin_view(0, "v:1", &prepared, text, spans);
    let _receiver = commands(&mut app, buffer, value, prepared.projection.clone());
    app.present_plugin_view(buffer);
    assert_eq!(
        app.active().selection.primary().head,
        prepared.projection.rows[0].from
    );
}
