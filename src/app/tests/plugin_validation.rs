// SPDX-License-Identifier: MPL-2.0

use super::*;
use crate::app::plugin_interaction::{Surface, ValidationIntent};
use crate::plugin::{
    application::CapturedContext,
    interaction::{Field, FieldValidation, Kind, ValidationStatus, Value},
};

fn field(id: &str, kind: Kind, validate: bool) -> Field {
    let mut field = Field::text(id, id.into());
    field.kind = kind;
    field.validate = validate;
    field.validation_message = Some("Use a registered account".into());
    field
}

fn fixture(fields: Vec<Field>) -> App {
    let mut app = App::new(Config::default(), None).unwrap();
    app.note_plugin_frontend(true);
    app.plugins.input = Some(Surface {
        owner: 7,
        handle: "u:test".into(),
        context: CapturedContext {
            foreground_allowed: true,
            action: None,
            pane: app.active_pane,
            buffer: app.active().buffer,
            terminal: None,
            attachment: app.plugins.attachment_generation,
            foreground: app.plugins.foreground_generation,
        },
        title: "Account".into(),
        values: fields.iter().map(Field::initial).collect(),
        fields,
        selected: 0,
        cursor: 0,
        error: false,
        picker: None,
        confirmation: false,
        validation: Default::default(),
    });
    app
}

fn form_snapshot(app: &App) -> crate::snapshot::OverlaySnapshot {
    app.overlay_snapshots()
        .into_iter()
        .find(|overlay| overlay.kind == crate::snapshot::OverlayKind::Prompt)
        .unwrap()
}

fn type_text(app: &mut App, value: &str) {
    app.handle_input(InputEvent::Text(value.into())).unwrap();
}

fn pending(app: &mut App) -> ValidationIntent {
    key(app, KeyCode::Enter, Modifiers::NONE);
    let intent = app.peek_plugin_validation().expect("validation intent");
    assert!(app.plugin_validation_pending(&intent));
    intent
}

fn respond(app: &mut App, intent: &ValidationIntent, status: ValidationStatus) -> bool {
    app.apply_plugin_validation(
        intent.owner,
        &intent.surface,
        &intent.revision,
        intent.foreground,
        &intent
            .fields
            .iter()
            .map(|field| FieldValidation {
                field: field.clone(),
                status,
            })
            .collect::<Vec<_>>(),
    )
}

#[test]
fn validation_requires_declarative_validity_and_physical_unmodified_enter() {
    let mut app = fixture(vec![field("account", Kind::Text, true)]);
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert!(app.peek_plugin_validation().is_none());
    assert!(app.plugins.input.as_ref().unwrap().error);
    type_text(&mut app, "name");
    for modifiers in [Modifiers::SHIFT, Modifiers::CONTROL, Modifiers::ALT] {
        key(&mut app, KeyCode::Enter, modifiers);
        assert!(app.peek_plugin_validation().is_none());
    }
    app.handle_replayed_input(InputEvent::Key(KeyStroke::new(
        KeyCode::Enter,
        Modifiers::NONE,
    )))
    .unwrap();
    assert!(app.peek_plugin_validation().is_none());
    assert_eq!(app.buffers[0].to_string(), "");
    let intent = pending(&mut app);
    assert!(respond(&mut app, &intent, ValidationStatus::Valid));
    assert!(app.plugins.input.is_none());
    assert_eq!(app.plugins.input_finished.len(), 1);
    let (_, context, submission) = &app.plugins.input_finished[0];
    assert!(submission.accepted);
    assert_eq!(context.foreground, intent.foreground);
    assert!(context.foreground_allowed);
    assert!(!respond(&mut app, &intent, ValidationStatus::Valid));
}

#[test]
fn fields_without_validation_keep_immediate_submission() {
    let mut app = fixture(vec![field("plain", Kind::Text, false)]);
    type_text(&mut app, "value");
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert!(app.peek_plugin_validation().is_none());
    assert_eq!(app.plugins.input_finished.len(), 1);
    assert!(app.plugins.input_finished[0].2.accepted);
}

#[test]
fn validation_includes_nonsecret_dependencies_and_only_opted_in_secret_values() {
    for opt_in in [false, true] {
        let mut app = fixture(vec![
            field("account", Kind::Text, true),
            field("password", Kind::Secret, opt_in),
        ]);
        type_text(&mut app, "public-account");
        key(&mut app, KeyCode::Tab, Modifiers::NONE);
        type_text(&mut app, "secret-never-render");
        let intent = pending(&mut app);
        assert_eq!(
            intent.values.get("account"),
            Some(&Value::Text("public-account".into()))
        );
        assert_eq!(intent.values.contains_key("password"), opt_in);
        assert_eq!(intent.fields.contains(&"password".into()), opt_in);
        let rendered = format!("{:?}", form_snapshot(&app));
        assert!(!rendered.contains("secret-never-render"));
        assert!(respond(&mut app, &intent, ValidationStatus::Invalid));
        let rendered = format!("{:?}", form_snapshot(&app));
        assert!(!rendered.contains("secret-never-render"));
        assert!(rendered.contains("Use a registered account"));
        let retried = pending(&mut app);
        assert!(respond(&mut app, &retried, ValidationStatus::Valid));
        assert!(app.plugins.input_finished[0].2.sensitive);
        assert_eq!(
            app.plugins.input_finished[0].2.values["password"],
            Value::Text("secret-never-render".into())
        );
    }
}

#[test]
fn editing_away_and_back_rejects_old_validation_and_retains_only_latest_enter() {
    let mut app = fixture(vec![field("account", Kind::Text, true)]);
    type_text(&mut app, "x");
    let first = pending(&mut app);
    key(&mut app, KeyCode::Backspace, Modifiers::NONE);
    type_text(&mut app, "x");
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert!(
        app.peek_plugin_validation().is_none(),
        "one in-flight request"
    );
    assert!(respond(&mut app, &first, ValidationStatus::Valid));
    assert!(app.plugins.input_finished.is_empty());
    let latest = app.peek_plugin_validation().unwrap();
    assert_ne!(latest.revision, first.revision);
    assert_eq!(latest.values, first.values);
    assert!(app.plugin_validation_pending(&latest));
    assert!(respond(&mut app, &latest, ValidationStatus::Valid));
    assert_eq!(app.plugins.input_finished.len(), 1);
}

#[test]
fn repeated_enter_does_not_reuse_the_older_foreground_intent() {
    let mut app = fixture(vec![field("account", Kind::Text, true)]);
    type_text(&mut app, "x");
    let first = pending(&mut app);
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert!(respond(&mut app, &first, ValidationStatus::Valid));
    assert!(app.plugins.input_finished.is_empty());
    let latest = app.peek_plugin_validation().unwrap();
    assert_eq!(latest.revision, first.revision);
    assert!(latest.foreground > first.foreground);
    assert!(app.plugin_validation_pending(&latest));
    respond(&mut app, &latest, ValidationStatus::Valid);
    assert_eq!(app.plugins.input_finished.len(), 1);
}

#[test]
fn cursor_pointer_replay_and_semantic_input_cancel_delayed_submit_authority() {
    for input in ["cursor", "pointer", "replay", "semantic", "native-pointer"] {
        let mut app = fixture(vec![field("account", Kind::Text, true)]);
        type_text(&mut app, "x");
        let intent = pending(&mut app);
        let pointer = PointerEvent {
            kind: PointerEventKind::Moved,
            column: 1,
            row: 1,
            modifiers: Modifiers::NONE,
        };
        match input {
            "cursor" => key(&mut app, KeyCode::Left, Modifiers::NONE),
            "pointer" => app.handle_input(InputEvent::Pointer(pointer)).unwrap(),
            "replay" => app
                .handle_replayed_input(InputEvent::Text("should not edit".into()))
                .unwrap(),
            "semantic" => {
                app.execute(
                    CommandInvocation::editor(EditorCommand::EnterNormalMode, Default::default())
                        .unwrap(),
                )
                .unwrap();
            }
            "native-pointer" => {
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
                app.handle_pointer(pointer, &view).unwrap();
            }
            _ => unreachable!(),
        }
        respond(&mut app, &intent, ValidationStatus::Valid);
        assert!(
            app.plugins
                .input_finished
                .iter()
                .all(|(_, _, submission)| !submission.accepted),
            "{input}"
        );
        assert_eq!(app.buffers[0].to_string(), "");
    }
}

#[test]
fn editing_an_unvalidated_dependency_invalidates_the_entire_form_revision() {
    let mut app = fixture(vec![
        field("account", Kind::Text, true),
        field("enabled", Kind::Boolean, false),
    ]);
    type_text(&mut app, "x");
    let first = pending(&mut app);
    key(&mut app, KeyCode::Tab, Modifiers::NONE);
    key(&mut app, KeyCode::Right, Modifiers::NONE);
    respond(&mut app, &first, ValidationStatus::Valid);
    let latest = pending(&mut app);
    assert_ne!(latest.revision, first.revision);
    assert_eq!(latest.values["enabled"], Value::Boolean(true));
}

#[test]
fn unavailable_and_malformed_validation_use_static_feedback_and_allow_explicit_retry() {
    for failure in ["unavailable", "failed", "malformed"] {
        let mut app = fixture(vec![field("account", Kind::Text, true)]);
        type_text(&mut app, "x");
        let intent = pending(&mut app);
        match failure {
            "unavailable" => {
                respond(&mut app, &intent, ValidationStatus::Unavailable);
            }
            "failed" => {
                app.plugin_validation_failed(
                    intent.owner,
                    &intent.surface,
                    &intent.revision,
                    intent.foreground,
                );
            }
            "malformed" => {
                app.apply_plugin_validation(
                    intent.owner,
                    &intent.surface,
                    &intent.revision,
                    intent.foreground,
                    &[],
                );
            }
            _ => unreachable!(),
        }
        let snapshot = form_snapshot(&app);
        assert!(snapshot.message.unwrap().contains("Validation unavailable"));
        assert!(app.plugins.input_finished.is_empty());
        let retry = pending(&mut app);
        respond(&mut app, &retry, ValidationStatus::Valid);
        assert_eq!(app.plugins.input_finished.len(), 1);
    }
}

#[test]
fn cancel_detach_takeover_and_owner_stop_settle_once_and_ignore_late_validation() {
    for reason in ["escape", "detach", "takeover", "owner"] {
        let mut app = fixture(vec![field("password", Kind::Secret, true)]);
        type_text(&mut app, "private");
        let intent = pending(&mut app);
        match reason {
            "escape" => key(&mut app, KeyCode::Escape, Modifiers::NONE),
            "detach" => app.note_plugin_frontend(false),
            "takeover" => app.mode = Mode::Command,
            "owner" => app.cancel_plugin_input(7, None),
            _ => unreachable!(),
        }
        app.sync_plugin_input();
        assert!(!respond(&mut app, &intent, ValidationStatus::Valid));
        assert_eq!(app.plugins.input_finished.len(), 1);
        let (_, context, submission) = &app.plugins.input_finished[0];
        assert!(!submission.accepted);
        assert!(!context.foreground_allowed);
        assert!(submission.values.is_empty());
        assert!(!submission.sensitive);
    }
}

#[test]
fn newer_explicit_retry_replaces_failed_attempt_feedback_when_admitted() {
    let mut app = fixture(vec![field("account", Kind::Text, true)]);
    type_text(&mut app, "x");
    let first = pending(&mut app);
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert!(app.plugin_validation_failed(
        first.owner,
        &first.surface,
        &first.revision,
        first.foreground
    ));
    let retry = app.peek_plugin_validation().unwrap();
    assert!(app.plugin_validation_pending(&retry));
    assert_eq!(
        form_snapshot(&app).message.as_deref(),
        Some("Checking fields…")
    );
    respond(&mut app, &retry, ValidationStatus::Valid);
    assert_eq!(app.plugins.input_finished.len(), 1);
}

#[test]
fn stale_completion_requests_redraw_and_invalid_feedback_does_not_steal_field_focus() {
    let mut app = fixture(vec![
        field("account", Kind::Text, true),
        field("enabled", Kind::Boolean, false),
    ]);
    type_text(&mut app, "x");
    let first = pending(&mut app);
    key(&mut app, KeyCode::Tab, Modifiers::NONE);
    let selected = app.plugins.input.as_ref().unwrap().selected;
    let cursor = app.plugins.input.as_ref().unwrap().cursor;
    respond(&mut app, &first, ValidationStatus::Invalid);
    assert_eq!(app.plugins.input.as_ref().unwrap().selected, selected);
    assert_eq!(app.plugins.input.as_ref().unwrap().cursor, cursor);
    let second = pending(&mut app);
    key(&mut app, KeyCode::Right, Modifiers::NONE);
    app.plugins.presentation_dirty = false;
    respond(&mut app, &second, ValidationStatus::Valid);
    assert!(app.plugins.presentation_dirty);
    assert!(app.plugins.input_finished.is_empty());
}

#[test]
fn pointer_only_cancellation_of_queued_validation_requests_redraw() {
    let mut app = fixture(vec![field("account", Kind::Text, true)]);
    type_text(&mut app, "x");
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert!(app.peek_plugin_validation().is_some());
    app.plugins.presentation_dirty = false;
    app.handle_input(InputEvent::Pointer(PointerEvent {
        kind: PointerEventKind::Moved,
        column: 1,
        row: 1,
        modifiers: Modifiers::NONE,
    }))
    .unwrap();
    assert!(app.peek_plugin_validation().is_none());
    assert!(app.plugins.presentation_dirty);
    assert!(form_snapshot(&app).message.is_none());
}
