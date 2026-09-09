// SPDX-License-Identifier: MPL-2.0

use super::*;
use crate::app::plugin_providers::ProviderInspectIntent;
use crate::buffer::{ProviderDocument, ProviderIdentity};

fn provider(app: &mut App, key: &str) -> usize {
    app.install_provider_document(
        Buffer::provider_document(
            ProviderDocument {
                identity: ProviderIdentity {
                    configured_plugin: "remote-app".into(),
                    provider: "files".into(),
                    key: key.into(),
                },
                label: "Notes 猫".into(),
                syntax_hint: None,
                version: "baseline-v1".into(),
                generation: "old-generation".into(),
                available: true,
                baseline_epoch: 0,
                uncertain: None,
            },
            "one\ntwo\n".into(),
        ),
        true,
    )
}

fn fixture() -> (App, usize) {
    let mut app = App::new(Config::default(), None).unwrap();
    app.note_plugin_frontend(true);
    let source = provider(&mut app, "opaque/source");
    (app, source)
}

fn inspect(app: &mut App) -> ProviderInspectIntent {
    app.execute_command("diff-remote").unwrap();
    assert!(!app.status_error, "{}", app.status);
    let mut intents = app.take_provider_inspect_intents();
    assert_eq!(intents.len(), 1);
    intents.pop_front().unwrap()
}

fn publish(app: &mut App, text: &str) -> usize {
    let intent = inspect(app);
    app.publish_provider_inspection(
        intent,
        "new-generation".into(),
        "remote-v2".into(),
        text.into(),
    )
    .unwrap()
}

#[test]
fn remote_inspection_queues_the_native_target_revision_and_foreground_without_mutation() {
    let (mut app, source) = fixture();
    let revision = app.buffers[source].revision();
    let panes = app.panes.len();
    let intent = inspect(&mut app);
    assert_eq!(intent.buffer, source);
    assert_eq!(intent.expected_revision, revision);
    assert_eq!(intent.context.buffer, source);
    assert_eq!(intent.context.pane, app.active_pane);
    assert!(intent.context.terminal.is_none());
    app.plugin_foreground(&intent.context).unwrap();
    assert_eq!(app.buffers[source].revision(), revision);
    assert_eq!(app.panes.len(), panes);
    assert!(app.diffs.is_empty());
    assert!(!app.document_mutation_pending(source));
    assert!(app.take_provider_inspect_intents().is_empty());
    assert!(crate::command::parse_colon_command("diff-remote unexpected").is_err());
}

#[test]
fn remote_inspection_snapshot_is_read_only_and_preserves_local_baseline_and_undo() {
    let (mut app, source) = fixture();
    app.apply_to_buffer(source, &Transaction::insert(4, "local\n"));
    app.buffers[source].commit_undo_group();
    let revision = app.buffers[source].revision();
    let history = app.buffers[source].history_len();
    let baseline = app.buffers[source].provider().unwrap().clone();
    let snapshot = publish(&mut app, "one\nremote 猫\ntwo\n");
    assert_eq!(app.diffs.len(), 1);
    assert_eq!(app.panes.len(), 2);
    assert_eq!(app.diffs[0].side(Side::Left).buffer, snapshot);
    assert_eq!(app.diffs[0].side(Side::Right).buffer, source);
    assert_eq!(app.active().buffer, source);
    assert!(app.buffers[snapshot].is_read_only());
    assert!(app.buffers[snapshot].path.is_none());
    assert!(
        app.buffers[snapshot]
            .pane_title()
            .starts_with("[remote snapshot]")
    );
    assert_eq!(app.buffers[snapshot].to_string(), "one\nremote 猫\ntwo\n");
    assert!(!app.apply_to_buffer(snapshot, &Transaction::insert(0, "forbidden")));
    assert!(!app.lsp_touch(snapshot));
    assert_eq!(app.buffers[source].provider(), Some(&baseline));
    assert_eq!(app.buffers[source].revision(), revision);
    assert_eq!(app.buffers[source].history_len(), history);
    assert_eq!(app.buffers[source].to_string(), "one\nlocal\ntwo\n");
    assert!(app.buffers[source].dirty);
    app.undo();
    assert_eq!(app.buffers[source].to_string(), "one\ntwo\n");
    assert!(!app.buffers[source].dirty);
}

#[test]
fn remote_inspection_refresh_reuses_snapshot_and_comparison_and_closed_snapshots_release_text() {
    let (mut app, source) = fixture();
    let snapshot = publish(&mut app, "old remote\n");
    let count = app.buffers.len();
    let intent = inspect(&mut app);
    let refreshed = app
        .publish_provider_inspection(
            intent,
            "another-generation".into(),
            "remote-v3".into(),
            "short\n".into(),
        )
        .unwrap();
    assert_eq!(refreshed, snapshot);
    assert_eq!(app.buffers.len(), count);
    assert_eq!(app.panes.len(), 2);
    assert_eq!(app.diffs.len(), 1);
    assert_eq!(app.buffers[snapshot].to_string(), "short\n");
    assert!(
        matches!(app.buffers[snapshot].generated_view_identity(), Some(GeneratedViewIdentity::ProviderSnapshot { source_buffer, generation, version }) if *source_buffer == source && generation == "another-generation" && version == "remote-v3")
    );
    app.diff_off();
    assert!(app.diffs.is_empty());
    assert_eq!(publish(&mut app, "one\ntwo\n"), snapshot);
    assert!(app.diffs[0].alignment().is_equal());
    app.host_close_buffer(snapshot, false).unwrap();
    assert!(app.host_buffer_is_closed(snapshot));
    assert_eq!(app.buffers[snapshot].len_bytes(), 0);
    // The shared comparison owner retires invalid pane pairs at frame preparation.
    app.prepare_view(FrameGeometry {
        screen: Rect {
            width: 80,
            height: 22,
            ..Rect::default()
        },
        editor: Rect {
            width: 80,
            height: 20,
            ..Rect::default()
        },
        status: Rect::default(),
        message: Rect::default(),
    });
    assert!(app.diffs.is_empty());
    let new_snapshot = publish(&mut app, "after reopen\n");
    assert_ne!(new_snapshot, snapshot);
    assert_eq!(app.provider_inspection_snapshot(source), Some(new_snapshot));
}

#[test]
fn remote_inspection_rejects_edits_or_stale_foreground_before_publication() {
    for change in [
        "edit",
        "input",
        "detach",
        "target",
        "pane",
        "closed",
        "terminal-context",
    ] {
        let (mut app, source) = fixture();
        let mut intent = inspect(&mut app);
        match change {
            "edit" => {
                app.apply_to_buffer(source, &Transaction::insert(0, "newer "));
            }
            "input" => key(&mut app, KeyCode::Right, Modifiers::NONE),
            "detach" => app.note_plugin_frontend(false),
            "target" => {
                provider(&mut app, "other-resource");
            }
            "pane" => app.split(Axis::Horizontal, None).unwrap(),
            "closed" => app.host_close_buffer(source, false).unwrap(),
            "terminal-context" => {
                intent.context.terminal = Some(crate::terminal::TerminalId::from_raw(99));
            }
            _ => unreachable!(),
        }
        let buffers = app.buffers.len();
        let panes = app.panes.len();
        assert!(
            app.publish_provider_inspection(
                intent,
                "generation".into(),
                "version".into(),
                "remote\n".into()
            )
            .is_err(),
            "{change}"
        );
        assert_eq!(app.buffers.len(), buffers, "{change}");
        assert_eq!(app.panes.len(), panes, "{change}");
        assert!(app.diffs.is_empty(), "{change}");
        assert!(
            app.provider_inspection_snapshot(source).is_none(),
            "{change}"
        );
    }
}

#[test]
fn remote_inspection_of_uncertain_unavailable_source_does_not_rebind_or_reconcile() {
    let (mut app, source) = fixture();
    app.apply_to_buffer(source, &Transaction::insert(0, "uncertain "));
    let save = app.buffers[source].prepare_provider_save().unwrap();
    app.buffers[source].mark_provider_uncertain(save);
    app.buffers[source].provider_mut().unwrap().available = false;
    let before = app.buffers[source].provider().unwrap().clone();
    let text = app.buffers[source].to_string();
    let snapshot = publish(&mut app, &text);
    assert_eq!(app.buffers[source].provider(), Some(&before));
    assert!(app.buffers[source].dirty);
    assert!(app.buffers[source].prepare_provider_save().is_err());
    assert_eq!(app.buffers[snapshot].to_string(), text);
    assert!(app.diffs[0].alignment().is_equal());
}

#[test]
fn remote_inspection_refuses_unrelated_comparison_without_changing_it() {
    let (mut app, source) = fixture();
    let intent = inspect(&mut app);
    let source_pane = app.active_pane;
    app.diff_this();
    app.split(Axis::Horizontal, None).unwrap();
    let unrelated = provider(&mut app, "other-resource");
    app.diff_this();
    app.activate_pane(source_pane);
    assert_eq!(app.active().buffer, source);
    assert!(app.diffs[0].has_buffer(unrelated));
    let panes = app.panes.len();
    let buffers = app.buffers.len();
    let error = app
        .publish_provider_inspection(
            intent,
            "generation".into(),
            "version".into(),
            "remote".into(),
        )
        .unwrap_err();
    assert!(error.to_string().contains("already being compared"));
    assert_eq!(app.panes.len(), panes);
    assert_eq!(app.buffers.len(), buffers);
    assert_eq!(app.diffs.len(), 1);
    assert!(app.diffs[0].has_buffer(unrelated));
    app.execute_command("diff-remote").unwrap();
    assert!(app.status_error);
    assert!(app.take_provider_inspect_intents().is_empty());
}

#[test]
fn remote_inspection_does_not_reuse_a_snapshot_pane_covered_by_a_terminal() {
    let (mut app, source) = fixture();
    let snapshot = publish(&mut app, "first remote\n");
    let snapshot_pane = app.diffs[0].side(Side::Left).pane;
    let intent = inspect(&mut app);
    let terminal = crate::terminal::TerminalId::from_raw(99);
    app.panes.get_mut(&snapshot_pane).unwrap().terminal = Some(terminal);
    assert!(
        app.publish_provider_inspection(
            intent,
            "generation".into(),
            "version".into(),
            "unexpected refresh\n".into()
        )
        .is_err()
    );
    assert_eq!(app.buffers[snapshot].to_string(), "first remote\n");
    assert_eq!(app.panes.len(), 2);
    app.diff_off();
    assert_eq!(publish(&mut app, "second remote\n"), snapshot);
    assert_eq!(app.panes.len(), 3);
    assert_eq!(app.panes[&snapshot_pane].terminal, Some(terminal));
    for side in [Side::Left, Side::Right] {
        let side = app.diffs[0].side(side);
        assert_ne!(side.pane, snapshot_pane);
        assert!(app.panes[&side.pane].terminal.is_none());
    }
    assert_eq!(app.active().buffer, source);
}

#[test]
fn remote_inspection_queue_is_bounded_and_size_refusal_never_publishes() {
    let (mut app, _) = fixture();
    app.execute_command("diff-remote").unwrap();
    app.execute_command("diff-remote").unwrap();
    assert!(app.status_error);
    assert_eq!(app.plugins.provider_inspect_intents.len(), 1);
    for index in 1..5 {
        provider(&mut app, &format!("resource-{index}"));
        app.execute_command("diff-remote").unwrap();
        assert_eq!(app.status_error, index == 4);
    }
    assert_eq!(app.take_provider_inspect_intents().len(), 4);

    let (mut app, source) = fixture();
    let intent = inspect(&mut app);
    let buffers = app.buffers.len();
    assert!(
        app.publish_provider_inspection(
            intent,
            "generation".into(),
            "version".into(),
            "x".repeat(MAX_DIFF_BYTES + 1)
        )
        .is_err()
    );
    assert_eq!(app.buffers.len(), buffers);
    assert_eq!(app.panes.len(), 1);
    app.apply_to_buffer(source, &Transaction::insert(0, "x".repeat(MAX_DIFF_BYTES)));
    app.execute_command("diff-remote").unwrap();
    assert!(app.status_error);
    assert!(app.take_provider_inspect_intents().is_empty());
    assert!(app.provider_inspection_snapshot(source).is_none());
}
