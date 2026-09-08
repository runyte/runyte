// SPDX-License-Identifier: MPL-2.0

use super::*;
use crate::app::plugin_providers::ProviderSaveIntent;
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
                label: "Remote 猫 notes".into(),
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

fn fixture(command: &str) -> (App, ProviderSaveIntent) {
    let mut app = App::new(Config::default(), None).unwrap();
    app.note_plugin_frontend(true);
    provider(&mut app, "opaque-do-not-display");
    app.execute_command(command).unwrap();
    let intent = app.take_provider_save_intents().pop_front().unwrap();
    (app, intent)
}

fn show(app: &mut App, intent: &ProviderSaveIntent, atomic: bool) -> anyhow::Result<()> {
    app.show_provider_overwrite(
        "j:test:1".into(),
        intent.buffer,
        intent.expected_revision,
        intent.context.clone(),
        "Remote 猫 notes".into(),
        atomic,
    )
}

#[test]
fn provider_overwrite_snapshot_names_race_and_atomicity_without_mutating_text() {
    for atomic in [false, true] {
        let (mut app, intent) = fixture("write");
        let revision = app.confirmation_revision;
        show(&mut app, &intent, atomic).unwrap();
        assert!(app.has_native_input_overlay());
        assert_ne!(app.confirmation_revision, revision);
        let snapshot = confirmation_snapshot(&app);
        let message = snapshot.message.unwrap();
        assert!(message.contains("Remote 猫 notes"));
        assert!(message.contains("cannot guarantee conditional writes"));
        assert!(
            message.contains("can still be overwritten") || message.contains("can be overwritten")
        );
        assert_eq!(message.contains("not atomic"), !atomic);
        assert_eq!(message.contains("partial content"), !atomic);
        assert!(!message.contains("opaque-do-not-display"));
        assert_eq!(app.buffers[intent.buffer].to_string(), "original  \n");
        let status = app.status.clone();
        app.report_completed_action("write", "saved", CommandOutcome::Completed);
        assert_eq!(app.status, status);
    }
}

#[test]
fn provider_overwrite_accepts_only_physical_unmodified_enter_once() {
    let (mut app, intent) = fixture("write");
    show(&mut app, &intent, true).unwrap();
    for input in [
        InputEvent::Text("pasted\n".into()),
        InputEvent::Key(KeyStroke::new(KeyCode::Enter, Modifiers::SHIFT)),
        InputEvent::Key(KeyStroke::new(KeyCode::Enter, Modifiers::CONTROL)),
        InputEvent::Key(KeyStroke::char('y')),
    ] {
        app.handle_input(input).unwrap();
        assert!(app.plugins.provider_overwrite.is_some());
        assert!(app.take_provider_overwrite_decisions().is_empty());
    }
    app.handle_replayed_input(InputEvent::Key(KeyStroke::new(
        KeyCode::Enter,
        Modifiers::NONE,
    )))
    .unwrap();
    assert!(app.take_provider_overwrite_decisions().is_empty());
    assert!(app.plugins.provider_overwrite.is_some());
    let foreground = app.plugins.foreground_generation;
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert!(app.plugins.provider_overwrite.is_none());
    let mut decisions = app.take_provider_overwrite_decisions();
    assert_eq!(decisions.len(), 1);
    let decision = decisions.pop().unwrap();
    assert_eq!(decision.job, "j:test:1");
    let context = decision.context.unwrap();
    assert!(context.foreground > foreground);
    app.plugin_foreground(&context).unwrap();
    assert_eq!(context.buffer, intent.buffer);
    assert!(app.take_provider_overwrite_decisions().is_empty());
    assert_eq!(app.buffers[intent.buffer].to_string(), "original  \n");
    assert!(app.document_mutation_pending(intent.buffer));
}

#[test]
fn provider_overwrite_pointer_input_cannot_approve_or_retarget() {
    let (mut app, intent) = fixture("write");
    show(&mut app, &intent, true).unwrap();
    let pane = app.active_pane;
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
    for kind in [
        PointerEventKind::Down(PointerButton::Left),
        PointerEventKind::Up(PointerButton::Left),
        PointerEventKind::ScrollDown,
        PointerEventKind::Moved,
    ] {
        let event = PointerEvent {
            kind,
            column: 2,
            row: 3,
            modifiers: Modifiers::NONE,
        };
        app.handle_pointer(event, &view).unwrap();
        app.handle_input(InputEvent::Pointer(event)).unwrap();
        assert!(app.plugins.provider_overwrite.is_some());
        assert!(app.take_provider_overwrite_decisions().is_empty());
        assert_eq!(app.active_pane, pane);
        assert_eq!(app.active().buffer, intent.buffer);
    }
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert!(app.take_provider_overwrite_decisions()[0].context.is_some());
}

#[test]
fn provider_overwrite_cancel_and_host_clear_never_issue_approval() {
    for key_code in [
        KeyStroke::new(KeyCode::Escape, Modifiers::NONE),
        KeyStroke::new(KeyCode::Char('c'), Modifiers::CONTROL),
        KeyStroke::char(' '),
    ] {
        let (mut app, intent) = fixture("write");
        show(&mut app, &intent, true).unwrap();
        app.handle_key(key_code).unwrap();
        assert!(app.plugins.provider_overwrite.is_none());
        let decisions = app.take_provider_overwrite_decisions();
        assert_eq!(decisions.len(), 1);
        assert!(decisions[0].context.is_none());
        assert_eq!(app.buffers[intent.buffer].to_string(), "original  \n");
    }
    let (mut app, intent) = fixture("write");
    show(&mut app, &intent, true).unwrap();
    app.clear_provider_overwrite("other-job");
    assert!(app.plugins.provider_overwrite.is_some());
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    app.clear_provider_overwrite("j:test:1");
    assert!(app.take_provider_overwrite_decisions().is_empty());
}

#[test]
fn provider_overwrite_stale_surface_cancels_without_redirecting_approval() {
    for change in [
        "edit",
        "detach",
        "target",
        "overlay",
        "semantic",
        "recording",
    ] {
        let (mut app, intent) = fixture("write");
        show(&mut app, &intent, true).unwrap();
        match change {
            "edit" => {
                app.apply_to_buffer(intent.buffer, &Transaction::insert(0, "newer "));
            }
            "detach" => app.note_plugin_frontend(false),
            "target" => {
                provider(&mut app, "other-resource");
            }
            "overlay" => app.buffer_discard_confirmation = Some(intent.buffer),
            "semantic" => {
                app.execute_command("write").unwrap();
            }
            "recording" => app.start_macro_recording('q'),
            _ => unreachable!(),
        }
        let text = app.active_buffer().to_string();
        key(&mut app, KeyCode::Enter, Modifiers::NONE);
        assert!(app.plugins.provider_overwrite.is_none(), "{change}");
        let decisions = app.take_provider_overwrite_decisions();
        assert_eq!(decisions.len(), 1, "{change}");
        assert!(decisions[0].context.is_none(), "{change}");
        assert_eq!(app.active_buffer().to_string(), text, "{change}");
        if change == "overlay" {
            assert!(app.buffer_discard_confirmation.is_some());
        }
    }
}

#[test]
fn stale_provider_overwrite_returns_the_triggering_key_to_the_editor() {
    let (mut app, intent) = fixture("write");
    show(&mut app, &intent, true).unwrap();
    app.buffers[intent.buffer].apply(&Transaction::insert(0, "newer "));
    let before = app.active().selection.primary().head;
    key(&mut app, KeyCode::Char('l'), Modifiers::NONE);
    assert_eq!(app.active().selection.primary().head, before + 1);
    assert!(app.plugins.provider_overwrite.is_none());
    let decisions = app.take_provider_overwrite_decisions();
    assert_eq!(decisions.len(), 1);
    assert!(decisions[0].context.is_none());
    assert_eq!(app.active_buffer().to_string(), "newer original  \n");
}

#[test]
fn provider_overwrite_macro_origin_remains_ineligible_after_recording_or_replay_ends() {
    for replay in [false, true] {
        let mut app = App::new(Config::default(), None).unwrap();
        app.note_plugin_frontend(true);
        provider(&mut app, "macro-source");
        if replay {
            app.macros
                .insert('q', vec![InputEvent::Key(KeyStroke::char('h'))]);
            app.replay_macro('q', 1).unwrap();
        } else {
            app.start_macro_recording('q');
        }
        app.execute_command("write").unwrap();
        let mut intent = app.take_provider_save_intents().pop_front().unwrap();
        assert!(!intent.context.foreground_allowed);
        app.recording_macro = None;
        app.cancel_macro_replay();
        intent.context.foreground = app.plugins.foreground_generation;
        assert!(show(&mut app, &intent, true).is_err());
        assert!(app.plugins.provider_overwrite.is_none());
        assert!(app.take_provider_overwrite_decisions().is_empty());
    }
}

#[test]
fn provider_overwrite_reoffer_requires_another_enter_and_close_uses_refreshed_context() {
    let (mut app, mut intent) = fixture("wbc");
    show(&mut app, &intent, true).unwrap();
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert!(show(&mut app, &intent, true).is_err());
    let context = app
        .take_provider_overwrite_decisions()
        .pop()
        .unwrap()
        .context
        .unwrap();
    intent.context = context.clone();
    show(&mut app, &intent, true).unwrap();
    assert!(app.take_provider_overwrite_decisions().is_empty());
    assert!(!app.host_buffer_is_closed(intent.buffer));
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    let context = app
        .take_provider_overwrite_decisions()
        .pop()
        .unwrap()
        .context
        .unwrap();
    let continuation = app
        .refresh_provider_save_continuation(intent.continuation, &context)
        .unwrap();
    let snapshot = app.buffers[intent.buffer].prepare_provider_save().unwrap();
    assert!(app.buffers[intent.buffer].accept_provider_save(snapshot, "v2".into()));
    app.plugins.document_saves.remove(&intent.buffer);
    app.finish_provider_save_continuation(Some(continuation));
    assert!(app.host_buffer_is_closed(intent.buffer));
    assert!(!app.should_quit);
}
