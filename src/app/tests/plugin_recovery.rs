// SPDX-License-Identifier: MPL-2.0

use super::*;
use crate::{
    app::plugin_recovery::ProviderReloadIntent,
    buffer::{PreparedProviderReload, ProviderDocument, ProviderIdentity, ProviderReloadChoice},
};
use std::sync::atomic::AtomicBool;

fn fixture(dirty: bool) -> (App, usize) {
    let mut app = App::new(Config::default(), None).unwrap();
    app.note_plugin_frontend(true);
    let buffer = app.install_provider_document(
        Buffer::provider_document(
            ProviderDocument {
                identity: ProviderIdentity {
                    configured_plugin: "remote".into(),
                    provider: "files".into(),
                    key: "opaque-never-display".into(),
                },
                label: "Notes 猫".into(),
                syntax_hint: None,
                version: "v1".into(),
                generation: "g1".into(),
                available: true,
                baseline_epoch: 0,
                uncertain: None,
            },
            "original\n".into(),
        ),
        true,
    );
    if dirty {
        app.apply_to_buffer(buffer, &Transaction::insert(0, "local "));
    }
    (app, buffer)
}

fn queue(app: &mut App) -> ProviderReloadIntent {
    app.execute_command("reload").unwrap();
    assert!(!app.status_error, "{}", app.status);
    app.take_provider_reload_intents().pop_front().unwrap()
}

fn prepare(app: &App, buffer: usize, remote: &str) -> PreparedProviderReload {
    app.buffers[buffer]
        .provider_reload_source()
        .unwrap()
        .prepare(
            remote.into(),
            "g2".into(),
            "v2".into(),
            true,
            &AtomicBool::new(false),
        )
        .unwrap()
}

fn show(app: &mut App, intent: ProviderReloadIntent) {
    app.show_provider_reload("j:reload".into(), intent, "Notes 猫".into())
        .unwrap();
}

#[test]
fn provider_reload_queue_reserves_target_and_preserves_live_text_until_host_admission() {
    let (mut app, buffer) = fixture(true);
    let before = app.buffers[buffer].revision();
    let intent = queue(&mut app);
    assert_eq!(intent.expected_revision, before);
    assert_eq!(intent.identity.key, "opaque-never-display");
    assert_eq!(intent.generation, "g1");
    assert_eq!(intent.epoch, 0);
    assert!(app.document_mutation_pending(buffer));
    assert!(app.queue_provider_reload().is_err());
    assert_eq!(app.buffers[buffer].to_string(), "local original\n");
    assert_eq!(app.buffers[buffer].provider().unwrap().version, "v1");
    app.execute_command("write").unwrap();
    assert!(app.take_provider_save_intents().is_empty());
}

#[test]
fn provider_reload_choice_snapshot_is_metadata_only_and_physical_enter_is_required() {
    let (mut app, buffer) = fixture(true);
    let intent = queue(&mut app);
    show(&mut app, intent);
    let overlays = app.overlay_snapshots();
    let overlay = overlays
        .iter()
        .find(|overlay| overlay.title == "Reload remote document")
        .unwrap();
    assert_eq!(overlay.purpose, crate::snapshot::OverlayPurpose::Choice);
    assert_eq!(overlay.input, crate::snapshot::OverlayInput::None);
    assert_eq!(overlay.rows.len(), 3);
    assert_eq!(overlay.selected, Some(2));
    assert_eq!(overlay.rows[0].label, "Reload remote text");
    assert!(overlay.message.as_ref().unwrap().contains("Notes 猫"));
    assert!(!format!("{overlay:?}").contains("opaque-never-display"));
    for input in [
        InputEvent::Text("do not insert\n".into()),
        InputEvent::Key(KeyStroke::new(KeyCode::Enter, Modifiers::SHIFT)),
        InputEvent::Key(KeyStroke::new(KeyCode::Enter, Modifiers::CONTROL)),
    ] {
        app.handle_input(input).unwrap();
        assert!(app.take_provider_reload_decisions().is_empty());
    }
    app.handle_replayed_input(InputEvent::Key(KeyStroke::new(
        KeyCode::Enter,
        Modifiers::NONE,
    )))
    .unwrap();
    assert!(app.take_provider_reload_decisions().is_empty());
    let foreground = app.plugins.foreground_generation;
    key(&mut app, KeyCode::Up, Modifiers::NONE);
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    let decision = app.take_provider_reload_decisions().pop().unwrap();
    assert_eq!(decision.choice, Some(ProviderReloadChoice::KeepLocal));
    let context = decision.context.unwrap();
    assert!(context.foreground > foreground);
    app.plugin_foreground(&context).unwrap();
    assert_eq!(app.buffers[buffer].to_string(), "local original\n");
    assert!(app.take_provider_reload_decisions().is_empty());
}

#[test]
fn provider_reload_navigation_cancel_and_host_clear_settle_once_without_adoption() {
    for cancel in ["escape", "row", "host"] {
        let (mut app, buffer) = fixture(true);
        let intent = queue(&mut app);
        show(&mut app, intent);
        key(&mut app, KeyCode::Tab, Modifiers::NONE);
        key(&mut app, KeyCode::BackTab, Modifiers::SHIFT);
        assert_eq!(app.plugins.provider_reload.as_ref().unwrap().selected, 2);
        match cancel {
            "escape" => key(&mut app, KeyCode::Escape, Modifiers::NONE),
            "row" => {
                key(&mut app, KeyCode::Enter, Modifiers::NONE);
            }
            "host" => {
                app.clear_provider_reload("other");
                assert!(app.plugins.provider_reload.is_some());
                app.clear_provider_reload("j:reload");
            }
            _ => unreachable!(),
        }
        assert!(app.plugins.provider_reload.is_none());
        let decisions = app.take_provider_reload_decisions();
        assert_eq!(decisions.len(), usize::from(cancel != "host"));
        assert!(
            decisions
                .iter()
                .all(|decision| decision.choice.is_none() && decision.context.is_none())
        );
        assert!(app.take_provider_reload_decisions().is_empty());
        assert_eq!(app.buffers[buffer].provider().unwrap().version, "v1");
        assert_eq!(app.buffers[buffer].to_string(), "local original\n");
    }
}

#[test]
fn provider_reload_cancels_stale_context_and_binding_without_touching_baseline() {
    for change in [
        "edit-away-back",
        "binding",
        "epoch",
        "generation",
        "detach",
        "pane",
        "target",
        "terminal",
        "overlay",
        "recording",
        "semantic",
    ] {
        let (mut app, buffer) = fixture(true);
        let intent = queue(&mut app);
        show(&mut app, intent);
        match change {
            "edit-away-back" => {
                app.buffers[buffer].apply(&Transaction::insert(0, "x"));
                app.buffers[buffer].apply(&Transaction::delete(0, 1));
            }
            "binding" => app.buffers[buffer].provider_mut().unwrap().identity.key = "other".into(),
            "epoch" => app.buffers[buffer].provider_mut().unwrap().baseline_epoch += 1,
            "generation" => {
                app.buffers[buffer].provider_mut().unwrap().generation = "changed".into()
            }
            "detach" => app.note_plugin_frontend(false),
            "pane" => app.split(Axis::Horizontal, None).unwrap(),
            "target" => {
                let target = app.buffers.len();
                app.buffers.push(Buffer::scratch());
                app.syntax.push(None);
                app.switch_buffer(target);
                assert_ne!(app.active().buffer, buffer);
            }
            "terminal" => {
                app.active_mut().terminal = Some(crate::terminal::TerminalId::from_raw(99))
            }
            "overlay" => app.buffer_discard_confirmation = Some(buffer),
            "recording" => app.start_macro_recording('q'),
            "semantic" => {
                app.execute_command("path").unwrap();
            }
            _ => unreachable!(),
        }
        app.sync_provider_reload();
        assert!(app.plugins.provider_reload.is_none(), "{change}");
        let decisions = app.take_provider_reload_decisions();
        assert_eq!(decisions.len(), 1, "{change}");
        assert!(decisions[0].context.is_none(), "{change}");
        assert_eq!(app.buffers[buffer].provider().unwrap().version, "v1");
        assert_eq!(app.buffers[buffer].to_string(), "local original\n");
    }
}

#[test]
fn provider_reload_acceptance_preserves_old_history_and_undo_uses_new_remote_baseline() {
    let (mut app, buffer) = fixture(true);
    let intent = queue(&mut app);
    let prepared = prepare(&app, buffer, "remote 猫\r\n");
    show(&mut app, intent);
    key(&mut app, KeyCode::Down, Modifiers::NONE);
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    let decision = app.take_provider_reload_decisions().pop().unwrap();
    app.install_provider_reload(
        buffer,
        prepared,
        decision.choice.unwrap(),
        &decision.context.unwrap(),
    )
    .unwrap();
    assert_eq!(app.buffers[buffer].to_string(), "remote 猫\r\n");
    assert!(!app.buffers[buffer].dirty);
    assert_eq!(app.buffers[buffer].provider().unwrap().version, "v2");
    assert!(app.buffers[buffer].undo());
    assert_eq!(app.buffers[buffer].to_string(), "local original\n");
    assert!(app.buffers[buffer].dirty);
    assert_eq!(app.buffers[buffer].provider().unwrap().version, "v2");
    assert!(app.buffers[buffer].undo());
    assert_eq!(app.buffers[buffer].to_string(), "original\n");
    assert!(app.buffers[buffer].dirty);
    assert!(app.buffers[buffer].redo());
    assert!(app.buffers[buffer].redo());
    assert_eq!(app.buffers[buffer].to_string(), "remote 猫\r\n");
    assert!(!app.buffers[buffer].dirty);
}

#[test]
fn provider_reload_keep_local_preserves_revision_and_redo_and_clean_reload_needs_no_surface() {
    let (mut app, buffer) = fixture(true);
    app.buffers[buffer].apply(&Transaction::insert(0, "temporary "));
    app.buffers[buffer].undo();
    let intent = queue(&mut app);
    let revision = app.buffers[buffer].revision();
    let prepared = prepare(&app, buffer, "remote\n");
    app.install_provider_reload(
        buffer,
        prepared,
        ProviderReloadChoice::KeepLocal,
        &intent.context,
    )
    .unwrap();
    assert_eq!(app.buffers[buffer].revision(), revision);
    assert_eq!(app.buffers[buffer].to_string(), "local original\n");
    assert!(app.buffers[buffer].dirty);
    assert!(app.buffers[buffer].redo());
    assert_eq!(
        app.buffers[buffer].to_string(),
        "temporary local original\n"
    );
    let (mut clean, buffer) = fixture(false);
    let intent = queue(&mut clean);
    assert!(
        !clean.buffers[buffer]
            .provider_reload_source()
            .unwrap()
            .requires_choice
    );
    let prepared = prepare(&clean, buffer, "");
    clean
        .install_provider_reload(
            buffer,
            prepared,
            ProviderReloadChoice::ReloadRemote,
            &intent.context,
        )
        .unwrap();
    assert!(clean.buffers[buffer].text().is_empty());
    assert!(!clean.buffers[buffer].dirty);
    assert!(clean.plugins.provider_reload.is_none());
}

#[test]
fn provider_reload_install_rejects_post_approval_input_and_uncertain_cancellation_preserves_recovery()
 {
    let (mut app, buffer) = fixture(true);
    let save = app.buffers[buffer].prepare_provider_save().unwrap();
    assert!(app.buffers[buffer].mark_provider_uncertain(save));
    let intent = queue(&mut app);
    let prepared = prepare(&app, buffer, "remote\n");
    show(&mut app, intent);
    key(&mut app, KeyCode::Down, Modifiers::NONE);
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    let decision = app.take_provider_reload_decisions().pop().unwrap();
    app.plugins.foreground_generation += 1;
    assert!(
        app.install_provider_reload(
            buffer,
            prepared,
            decision.choice.unwrap(),
            &decision.context.unwrap()
        )
        .is_err()
    );
    assert!(app.buffers[buffer].provider().unwrap().uncertain.is_some());
    assert_eq!(app.buffers[buffer].provider().unwrap().version, "v1");
    assert_eq!(app.buffers[buffer].to_string(), "local original\n");
}

#[test]
fn provider_reload_pointer_and_unused_keys_preserve_surface_without_redraw_or_document_input() {
    let (mut app, buffer) = fixture(true);
    let intent = queue(&mut app);
    show(&mut app, intent);
    let view = app.prepare_view(FrameGeometry {
        screen: Rect {
            width: 80,
            height: 24,
            ..Rect::default()
        },
        editor: Rect {
            width: 80,
            height: 22,
            ..Rect::default()
        },
        status: Rect::default(),
        message: Rect::default(),
    });
    app.plugins.presentation_dirty = false;
    let revision = app.confirmation_revision;
    let pointer = PointerEvent {
        kind: PointerEventKind::Down(PointerButton::Left),
        column: 2,
        row: 3,
        modifiers: Modifiers::NONE,
    };
    app.handle_pointer(pointer, &view).unwrap();
    app.handle_input(InputEvent::Pointer(pointer)).unwrap();
    app.handle_input(InputEvent::Key(KeyStroke::char('x')))
        .unwrap();
    assert!(!app.plugins.presentation_dirty);
    assert_eq!(app.confirmation_revision, revision);
    assert!(app.take_provider_reload_decisions().is_empty());
    assert_eq!(app.buffers[buffer].to_string(), "local original\n");
    assert_eq!(app.plugins.provider_reload.as_ref().unwrap().selected, 2);
}

#[test]
fn provider_reload_macro_and_stale_queued_binding_are_refused_before_choice() {
    let (mut app, buffer) = fixture(true);
    app.start_macro_recording('q');
    assert!(app.queue_provider_reload().is_err());
    assert!(!app.document_mutation_pending(buffer));
    app.recording_macro = None;
    let intent = queue(&mut app);
    app.buffers[buffer].provider_mut().unwrap().baseline_epoch += 1;
    assert!(app.validate_provider_reload_intent(&intent).is_err());
    assert!(
        app.show_provider_reload("j:reload".into(), intent, "Notes".into())
            .is_err()
    );
    assert!(app.plugins.provider_reload.is_none());
}

#[test]
fn provider_reload_completion_replaces_only_the_original_action_echo() {
    let (mut app, _) = fixture(true);
    app.active_action_id = Some(77);
    let intent = queue(&mut app);
    app.action_feedback = Some(ActionFeedback {
        id: 77,
        spelling: ":reload".into(),
        text: ":reload (Reading remote text for reload)".into(),
        is_error: false,
    });
    show(&mut app, intent);
    app.active_action_id = Some(88);
    key(&mut app, KeyCode::Up, Modifiers::NONE);
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    let decision = app.take_provider_reload_decisions().pop().unwrap();
    assert_eq!(decision.context.as_ref().unwrap().action, Some(77));
    app.provider_reload_feedback(
        decision.context.unwrap().action,
        true,
        "Remote baseline accepted",
    );
    assert_eq!(
        app.action_feedback.as_ref().unwrap().text,
        ":reload (Remote baseline accepted)"
    );
    app.action_feedback.as_mut().unwrap().id = 88;
    app.action_feedback.as_mut().unwrap().text = "A newer action".into();
    app.provider_reload_feedback(Some(77), false, "Provider stopped");
    assert_eq!(app.action_feedback.as_ref().unwrap().text, "A newer action");
}

#[test]
fn provider_reload_retired_worker_protects_standalone_quit_until_actual_cleanup() {
    let (mut app, buffer) = fixture(false);
    app.plugins.provider_reload_cleanup = 1;
    assert!(!app.document_mutation_pending(buffer));
    assert_eq!(app.plugin_active_job_count(), 1);
    app.execute_command("q!").unwrap();
    assert!(!app.should_quit);
    app.plugins.provider_reload_cleanup = 0;
    app.execute_command("q!").unwrap();
    assert!(app.should_quit);
}

#[test]
fn provider_reload_choice_renders_all_rows_and_default_cancel_in_native_frames() {
    use ratatui::{Terminal, backend::TestBackend};
    for (width, height) in [(100, 24), (44, 14)] {
        let (mut app, _) = fixture(true);
        let intent = queue(&mut app);
        show(&mut app, intent);
        assert!(
            app.list.is_none(),
            "the reload surface owns metadata, not an ordinary list"
        );
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| {
                let prepared = app.prepare_view(crate::ui::frame_geometry(frame.area()));
                let snapshot = app.snapshot(&prepared);
                crate::ui::render_exact_colors_for_test(
                    frame,
                    &app,
                    &snapshot,
                    &crate::key_hints::KeyHintState::default(),
                );
            })
            .unwrap();
        let buffer = terminal.backend().buffer();
        let lines = (0..height)
            .map(|row| {
                (0..width)
                    .map(|column| buffer[(column, row)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>();
        let screen = lines.join("\n");
        for label in ["Reload remote", "Keep local edits", "Cancel"] {
            assert!(
                screen.contains(label),
                "{width}x{height} lacks {label}:\n{screen}"
            );
        }
        assert!(
            lines
                .iter()
                .any(|line| line.contains('▸') && line.contains("Cancel")),
            "default Cancel must be visibly selected:\n{screen}"
        );
    }
}

#[test]
fn provider_reload_terminal_feedback_redraws_without_a_matching_action_echo() {
    let (mut app, _) = fixture(false);
    app.plugins.presentation_dirty = false;
    app.action_feedback = None;
    app.provider_reload_feedback(Some(55), false, "Resource provider is unavailable");
    assert!(app.plugins.presentation_dirty);
    assert!(app.status.contains("Resource provider is unavailable"));
}
