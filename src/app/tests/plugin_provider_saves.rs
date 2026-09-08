// SPDX-License-Identifier: MPL-2.0

use super::*;
use crate::buffer::{ProviderDocument, ProviderIdentity};

fn provider(app: &mut App, key: &str) -> usize {
    app.install_provider_document(
        Buffer::provider_document(
            ProviderDocument {
                identity: ProviderIdentity {
                    configured_plugin: "remote".into(),
                    provider: "files".into(),
                    key: key.into(),
                },
                label: "Remote notes".into(),
                syntax_hint: None,
                version: "v1".into(),
                generation: "g1".into(),
                available: true,
                baseline_epoch: 0,
                uncertain: None,
            },
            "original  \n".into(),
        ),
        true,
    )
}

#[test]
fn native_provider_saves_queue_without_trimming_closing_or_completing_waits() {
    for command in [
        "write",
        "w",
        "save",
        "write-quit",
        "wq",
        "write-buffer-close",
        "wbc",
    ] {
        let mut app = App::new(Config::default(), None).unwrap();
        app.config.editor.trim_trailing_whitespace = true;
        let buffer = provider(&mut app, command);
        app.set_parent_wait_buffers(HashMap::from([(buffer, 1)]));
        let revision = app.buffers[buffer].revision();
        app.execute_command(command).unwrap();
        assert!(!app.should_quit, "{command}");
        assert!(!app.host_buffer_is_closed(buffer), "{command}");
        assert_eq!(app.active().buffer, buffer);
        assert_eq!(app.buffers[buffer].to_string(), "original  \n");
        assert_eq!(app.buffers[buffer].revision(), revision);
        assert!(!app.buffers[buffer].dirty);
        assert!(app.take_parent_wait_actions().is_empty());
        assert!(app.plugins.document_saves.contains(&buffer));
        let intents = app.take_provider_save_intents();
        assert_eq!(intents.len(), 1);
        assert_eq!(intents[0].buffer, buffer);
        assert_eq!(intents[0].expected_revision, revision);
        assert_eq!(
            intents[0].continuation.is_some(),
            !matches!(command, "write" | "w" | "save")
        );
        assert!(app.plugins.document_saves.contains(&buffer));
    }
}

#[test]
fn native_provider_save_queue_is_bounded_and_duplicates_do_not_add_continuations() {
    let mut app = App::new(Config::default(), None).unwrap();
    let first = provider(&mut app, "first");
    app.execute_command("write").unwrap();
    app.execute_command("wq").unwrap();
    assert_eq!(app.plugins.provider_save_intents.len(), 1);
    assert!(app.plugins.provider_save_intents[0].continuation.is_none());
    assert!(!app.should_quit);
    for index in 1..5 {
        let buffer = provider(&mut app, &format!("resource-{index}"));
        app.save_buffer(buffer, None, false).unwrap();
        assert_eq!(app.plugins.document_saves.contains(&buffer), index < 4);
    }
    assert_eq!(app.plugins.provider_save_intents.len(), 4);
    assert_eq!(app.plugins.document_saves.len(), 4);
    assert!(app.plugins.document_saves.contains(&first));
    assert!(app.status.contains("full"));
}

#[test]
fn refused_provider_saves_preserve_text_and_do_not_reserve_guards() {
    for refused in [
        "force",
        "path",
        "unavailable",
        "uncertain",
        "filesystem",
        "private",
    ] {
        let mut app = App::new(Config::default(), None).unwrap();
        app.config.editor.trim_trailing_whitespace = true;
        let buffer = provider(&mut app, refused);
        match refused {
            "force" => app.save_buffer(buffer, None, true).unwrap(),
            "path" => app
                .save_buffer(buffer, Some(PathBuf::from("must-not-create.txt")), false)
                .unwrap(),
            "unavailable" => {
                app.buffers[buffer].provider_mut().unwrap().available = false;
                app.save_buffer(buffer, None, false).unwrap();
            }
            "uncertain" => {
                app.buffers[buffer].mark_write_uncertain();
                app.save_buffer(buffer, None, false).unwrap();
            }
            "filesystem" => {
                app.plugins.filesystem_applying = true;
                app.save_buffer(buffer, None, false).unwrap();
            }
            "private" => assert!(app.host_save_buffer(buffer).is_err()),
            _ => unreachable!(),
        }
        assert_eq!(app.buffers[buffer].to_string(), "original  \n", "{refused}");
        assert!(app.plugins.provider_save_intents.is_empty(), "{refused}");
        assert!(!app.plugins.document_saves.contains(&buffer), "{refused}");
    }
}

#[test]
fn provider_save_hooks_run_only_on_admission_and_are_undoable() {
    let mut app = App::new(Config::default(), None).unwrap();
    app.config.editor.trim_trailing_whitespace = true;
    let buffer = provider(&mut app, "trim");
    app.execute_command("write").unwrap();
    assert_eq!(app.buffers[buffer].to_string(), "original  \n");
    let snapshot = app.prepare_provider_save_text(buffer).unwrap();
    assert_eq!(snapshot.text.to_string(), "original\n");
    assert_eq!(app.buffers[buffer].to_string(), "original\n");
    app.undo();
    assert_eq!(app.buffers[buffer].to_string(), "original  \n");

    app.buffers[buffer].mark_write_uncertain();
    assert!(app.prepare_provider_save_text(buffer).is_err());
    assert_eq!(app.buffers[buffer].to_string(), "original  \n");
}

#[test]
fn confirmed_provider_save_closes_only_the_requested_view_or_buffer() {
    for command in ["wq", "wbc"] {
        let mut app = App::new(Config::default(), None).unwrap();
        let original_pane = app.active_pane;
        app.split(Axis::Horizontal, None).unwrap();
        let provider_pane = app.active_pane;
        let buffer = provider(&mut app, command);
        app.execute_command(command).unwrap();
        let intent = app.take_provider_save_intents().pop_front().unwrap();
        let snapshot = app.prepare_provider_save_text(buffer).unwrap();
        app.finish_provider_save_continuation(intent.continuation);
        assert_eq!(app.panes.len(), 2);
        assert!(!app.host_buffer_is_closed(buffer));
        assert!(app.buffers[buffer].accept_provider_save(snapshot, "v2".into()));
        app.plugins.document_saves.remove(&buffer);
        app.finish_provider_save_continuation(intent.continuation);
        assert!(app.host_buffer_is_closed(buffer), "{command}");
        assert!(app.panes.contains_key(&original_pane));
        assert_eq!(app.panes.contains_key(&provider_pane), command == "wbc");
        assert!(!app.should_quit);
    }
}

#[test]
fn provider_save_continuations_suppress_stale_context_and_newer_edits() {
    for change in [
        "input",
        "attachment",
        "pane",
        "buffer",
        "dirty",
        "uncertain",
    ] {
        let mut app = App::new(Config::default(), None).unwrap();
        let buffer = provider(&mut app, change);
        app.execute_command("wbc").unwrap();
        let intent = app.take_provider_save_intents().pop_front().unwrap();
        app.plugins.document_saves.remove(&buffer);
        match change {
            "input" => key(&mut app, KeyCode::Left, Modifiers::NONE),
            "attachment" => app.note_plugin_frontend(true),
            "pane" => app.split(Axis::Horizontal, None).unwrap(),
            "buffer" => {
                let other = provider(&mut app, "different-resource");
                assert_ne!(app.active().buffer, buffer);
                assert_eq!(app.active().buffer, other);
            }
            "dirty" => {
                app.apply_to_buffer(buffer, &Transaction::insert(0, "newer "));
            }
            "uncertain" => app.buffers[buffer].mark_write_uncertain(),
            _ => unreachable!(),
        }
        app.finish_provider_save_continuation(intent.continuation);
        assert!(!app.host_buffer_is_closed(buffer), "{change}");
        assert!(!app.should_quit, "{change}");
    }
}

#[test]
fn provider_save_close_completes_wait_only_after_clean_confirmation() {
    for command in ["wq", "wbc"] {
        let mut app = App::new(Config::default(), None).unwrap();
        let buffer = provider(&mut app, command);
        app.set_parent_wait_buffers(HashMap::from([(buffer, 1)]));
        app.execute_command(command).unwrap();
        let intent = app.take_provider_save_intents().pop_front().unwrap();
        assert!(app.take_parent_wait_actions().is_empty());
        let snapshot = app.prepare_provider_save_text(buffer).unwrap();
        assert!(app.buffers[buffer].accept_provider_save(snapshot, "v2".into()));
        app.plugins.document_saves.remove(&buffer);
        app.finish_provider_save_continuation(intent.continuation);
        assert_eq!(app.take_parent_wait_actions(), vec![(buffer, false)]);
        assert!(!app.host_buffer_is_closed(buffer));
        assert!(!app.should_quit);
    }
}

#[test]
fn accepted_provider_snapshot_does_not_close_newer_edits_or_complete_wait() {
    let mut app = App::new(Config::default(), None).unwrap();
    app.config.editor.trim_trailing_whitespace = false;
    let buffer = provider(&mut app, "edited-during-upload");
    app.apply_to_buffer(buffer, &Transaction::insert(0, "saved "));
    app.set_parent_wait_buffers(HashMap::from([(buffer, 1)]));
    app.execute_command("wq").unwrap();
    let intent = app.take_provider_save_intents().pop_front().unwrap();
    let snapshot = app.prepare_provider_save_text(buffer).unwrap();
    app.apply_to_buffer(buffer, &Transaction::insert(0, "newer "));
    app.buffers[buffer].commit_undo_group();
    assert!(app.buffers[buffer].accept_provider_save(snapshot, "v2".into()));
    app.plugins.document_saves.remove(&buffer);
    app.finish_provider_save_continuation(intent.continuation);
    assert!(app.buffers[buffer].dirty);
    assert_eq!(app.buffers[buffer].to_string(), "newer saved original  \n");
    assert!(!app.host_buffer_is_closed(buffer));
    assert!(!app.should_quit);
    assert!(app.take_parent_wait_actions().is_empty());
    app.undo();
    assert_eq!(app.buffers[buffer].to_string(), "saved original  \n");
    assert!(!app.buffers[buffer].dirty);
    assert!(!app.should_quit);
    assert!(app.take_parent_wait_actions().is_empty());
}
