// SPDX-License-Identifier: MPL-2.0
use super::*;
use crate::{
    input::{InputEvent, KeyStroke},
    plugin::interaction::{Field, Kind, Value},
};
fn begin(
    host: &mut WorkspaceHost,
    output: &mut mpsc::Receiver<HostMessage>,
    serial: u64,
    fields: Vec<Field>,
) -> String {
    host.app.note_plugin_frontend(true);
    invoke(host, "plugin.app-0.open");
    let api::HostMessage::Request { id, .. } = next(output) else {
        panic!()
    };
    request(
        host,
        0,
        serial,
        api::Request::UiForm {
            invocation: id.clone(),
            title: "Connection".into(),
            fields,
        },
    );
    let api::HostMessage::Response {
        outcome:
            api::Response::Success {
                result: api::ResultValue::Surface { surface },
            },
        ..
    } = next(output)
    else {
        panic!()
    };
    host.application_message(
        0,
        api::ClientMessage::Response {
            id,
            outcome: api::CommandResponse::Success {
                result: api::CommandResult { job: None },
            },
        },
    )
    .unwrap();
    // Clear the command deadline. Input waiting itself has no deadline.
    assert!(matches!(
        output.try_recv().unwrap(),
        HostMessage::Deadline { after_ms: None, .. }
    ));
    surface
}
fn key(host: &mut WorkspaceHost, key: &str) {
    host.app.handle_key(KeyStroke::parse(key).unwrap()).unwrap();
}
#[test]
fn form_masks_secrets_in_private_frames_and_returns_unicode_only_to_owner() {
    let (_root, mut host) = host();
    let mut output = setup(&mut host, 0, &["interaction", "views"]);
    next(&mut output);
    let mut secret = Field::text("password", "Password".into());
    secret.kind = Kind::Secret;
    let mut choice = Field::text("transport", "Transport".into());
    choice.kind = Kind::Choice;
    choice.choices = vec!["sftp".into(), "ftps".into()];
    let surface = begin(
        &mut host,
        &mut output,
        1,
        vec![Field::text("name", "Name".into()), secret, choice],
    );
    key(&mut host, "Enter");
    assert!(host.app.plugins.input.as_ref().unwrap().error);
    host.app
        .handle_input(InputEvent::Text("é猫".into()))
        .unwrap();
    key(&mut host, "Tab");
    host.app
        .handle_input(InputEvent::Text("test-secret-🔑".into()))
        .unwrap();
    let geometry = crate::ui::frame_geometry(ratatui::layout::Rect::new(0, 0, 40, 12));
    let wire: crate::protocol::HostFrame = host.prepare_frame(geometry).into();
    let bytes = serde_json::to_vec(&wire).unwrap();
    assert!(!String::from_utf8_lossy(&bytes).contains("test-secret"));
    let restored: crate::protocol::HostFrame = serde_json::from_slice(&bytes).unwrap();
    let frame: crate::workspace::HostFrame = restored.try_into().unwrap();
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(40, 12)).unwrap();
    terminal
        .draw(|f| crate::ui::render_host_frame_exact_colors_for_test(f, &frame))
        .unwrap();
    assert!(
        terminal
            .backend()
            .buffer()
            .content
            .iter()
            .any(|c| c.symbol() == "•")
    );
    assert!(host.app.command.is_empty());
    key(&mut host, "Tab");
    key(&mut host, "Right");
    key(&mut host, "Enter");
    host.sync_plugin_observers();
    let api::HostMessage::InputRequest { id, params, .. } = next(&mut output) else {
        panic!()
    };
    assert_eq!(params.surface, surface);
    assert!(params.accepted);
    assert_eq!(
        params.values["password"],
        Value::Text("test-secret-🔑".into())
    );
    assert_eq!(params.values["name"], Value::Text("é猫".into()));
    assert_eq!(params.values["transport"], Value::Text("ftps".into()));
    host.app
        .plugin_foreground(&host.app.plugins.instances[&0].application.requests[&id])
        .unwrap();
    assert!(host.app.plugins.input.is_none());
    assert!(
        host.app
            .buffers
            .iter()
            .all(|b| !b.to_string().contains("test-secret"))
    );
}
#[test]
fn input_ownership_busy_cancellation_detach_and_stop_settle_once() {
    for cancel in ["dismiss", "escape", "detach", "stop"] {
        let (_root, mut host) = host();
        let mut output = setup(&mut host, 0, &["interaction"]);
        next(&mut output);
        let mut other = setup(&mut host, 1, &["interaction"]);
        next(&mut other);
        let surface = begin(
            &mut host,
            &mut output,
            1,
            vec![Field::text("name", "Name".into())],
        );
        request(
            &mut host,
            1,
            1,
            api::Request::UiDismiss {
                surface: surface.clone(),
            },
        );
        next(&mut other);
        assert!(host.app.plugins.input.is_some());
        invoke(&mut host, "plugin.app-1.open");
        let api::HostMessage::Request { id, .. } = next(&mut other) else {
            panic!()
        };
        request(
            &mut host,
            1,
            2,
            api::Request::UiPick {
                invocation: id,
                title: "Other".into(),
                choices: vec!["one".into()],
            },
        );
        assert!(matches!(
            next(&mut other),
            api::HostMessage::Response {
                outcome: api::Response::Failure {
                    error: api::Error {
                        code: api::ErrorCode::Busy,
                        ..
                    }
                },
                ..
            }
        ));
        match cancel {
            "dismiss" => {
                request(&mut host, 0, 2, api::Request::UiDismiss { surface });
                next(&mut output);
            }
            "escape" => key(&mut host, "Esc"),
            "detach" => host.app.note_plugin_frontend(false),
            _ => host.stop_plugin(0, "stopped by user"),
        }
        host.sync_plugin_observers();
        assert!(host.app.plugins.input.is_none());
        if cancel != "stop" {
            let api::HostMessage::InputRequest { params, .. } = next(&mut output) else {
                panic!()
            };
            assert!(!params.accepted);
            assert!(params.values.is_empty());
            assert_eq!(
                host.app.plugins.instances[&0].application.retained_payload,
                0
            );
            assert!(matches!(
                output.try_recv().unwrap(),
                HostMessage::Deadline {
                    after_ms: Some(10000),
                    ..
                }
            ));
            host.sync_plugin_observers();
            assert!(output.try_recv().is_err());
        }
    }
}
#[test]
fn form_validation_and_editing_enforce_bounds_before_acceptance() {
    let (_root, mut host) = host();
    let mut output = setup(&mut host, 0, &["interaction"]);
    next(&mut output);
    let mut field = Field::text("name", "Name".into());
    field.minimum_length = 2;
    field.maximum_length = 3;
    begin(&mut host, &mut output, 1, vec![field.clone()]);
    host.app
        .handle_input(InputEvent::Text("é猫a".into()))
        .unwrap();
    key(&mut host, "Left");
    key(&mut host, "Backspace");
    key(&mut host, "Delete");
    assert_eq!(
        host.app.plugins.input.as_ref().unwrap().values[0],
        Value::Text("é".into())
    );
    key(&mut host, "Enter");
    assert!(host.app.plugins.input.is_some());
    host.app
        .handle_input(InputEvent::Text("\nunsafe".into()))
        .unwrap();
    assert_eq!(
        host.app.plugins.input.as_ref().unwrap().values[0],
        Value::Text("é".into())
    );
    host.app
        .handle_input(InputEvent::Text("猫xmore".into()))
        .unwrap();
    assert_eq!(
        host.app.plugins.input.as_ref().unwrap().values[0],
        Value::Text("é".into())
    );
    host.app
        .handle_input(InputEvent::Text("猫".into()))
        .unwrap();
    key(&mut host, "Enter");
    assert!(host.app.plugins.input.is_none());
    for fields in [vec![], vec![field.clone(), field.clone()]] {
        assert!(crate::plugin::interaction::validate("Form", &fields).is_err());
    }
    field.kind = Kind::Choice;
    field.choices = vec!["duplicate".into(); 2];
    assert!(crate::plugin::interaction::validate("Form", &[field]).is_err());
}

#[test]
fn picker_filters_visible_choices_and_cancellation_cannot_reopen_input() {
    let (_root, mut host) = host();
    let mut output = setup(&mut host, 0, &["interaction"]);
    next(&mut output);
    host.app.note_plugin_frontend(true);
    invoke(&mut host, "plugin.app-0.open");
    let api::HostMessage::Request { id, .. } = next(&mut output) else {
        panic!()
    };
    request(
        &mut host,
        0,
        1,
        api::Request::UiPick {
            invocation: id,
            title: "Pick".into(),
            choices: vec!["sftp".into(), "ftp".into(), "ftps".into()],
        },
    );
    next(&mut output);
    host.app
        .handle_input(InputEvent::Text("ftp".into()))
        .unwrap();
    let overlays = host.app.overlay_snapshots();
    let overlay = overlays.last().unwrap();
    assert_eq!(overlay.purpose, crate::snapshot::OverlayPurpose::Picker);
    assert_eq!(overlay.rows.len(), 3);
    key(&mut host, "Down");
    key(&mut host, "Left");
    assert_eq!(
        host.app
            .plugins
            .input
            .as_ref()
            .unwrap()
            .picker
            .as_ref()
            .unwrap()
            .selected,
        1
    );
    key(&mut host, "Enter");
    host.sync_plugin_observers();
    let api::HostMessage::InputRequest { params, .. } = next(&mut output) else {
        panic!()
    };
    assert!(params.accepted);
    assert!(!params.values.is_empty());
    // A cancelled surface still receives a terminal callback, but no presentation grant.
    while output.try_recv().is_ok() {}
    begin(
        &mut host,
        &mut output,
        2,
        vec![Field::text("name", "Name".into())],
    );
    key(&mut host, "Esc");
    host.sync_plugin_observers();
    let api::HostMessage::InputRequest { id, params, .. } = next(&mut output) else {
        panic!()
    };
    assert!(!params.accepted);
    request(
        &mut host,
        0,
        3,
        api::Request::UiConfirm {
            invocation: id,
            title: "Again".into(),
            message: "Continue?".into(),
        },
    );
    assert!(matches!(
        next(&mut output),
        api::HostMessage::Response {
            outcome: api::Response::Failure {
                error: api::Error {
                    code: api::ErrorCode::ContextChanged,
                    ..
                }
            },
            ..
        }
    ));
    assert!(host.app.plugins.input.is_none());
}
#[test]
fn native_input_takeover_cancels_plugin_surface_without_consuming_native_input() {
    for command_mode in [false, true] {
        let (_root, mut host) = host();
        let mut output = setup(&mut host, 0, &["interaction"]);
        next(&mut output);
        begin(
            &mut host,
            &mut output,
            1,
            vec![Field::text("name", "Name".into())],
        );
        if command_mode {
            host.app.mode = crate::app::Mode::Command;
        } else {
            host.app.list = Some(crate::picker::ListPicker::new("Native", vec![]));
        }
        host.sync_plugin_observers();
        assert!(host.app.plugins.input.is_none());
        let api::HostMessage::InputRequest { params, .. } = next(&mut output) else {
            panic!()
        };
        assert!(!params.accepted);
        if command_mode {
            assert_eq!(host.app.mode, crate::app::Mode::Command);
        } else {
            assert!(host.app.list.is_some());
        }
    }
}

#[test]
fn simple_confirmation_uses_native_enter_and_escape_semantics() {
    for accept in [false, true] {
        let (_root, mut host) = host();
        let mut output = setup(&mut host, 0, &["interaction"]);
        next(&mut output);
        host.app.note_plugin_frontend(true);
        invoke(&mut host, "plugin.app-0.open");
        let api::HostMessage::Request { id, .. } = next(&mut output) else {
            panic!()
        };
        request(
            &mut host,
            0,
            1,
            api::Request::UiConfirm {
                invocation: id,
                title: "Review".into(),
                message: "Continue operation?".into(),
            },
        );
        next(&mut output);
        let overlays = host.app.overlay_snapshots();
        let overlay = overlays.last().unwrap();
        assert_eq!(
            overlay.purpose,
            crate::snapshot::OverlayPurpose::Confirmation
        );
        assert_eq!(overlay.input, crate::snapshot::OverlayInput::None);
        key(&mut host, if accept { "Enter" } else { "Esc" });
        host.sync_plugin_observers();
        let api::HostMessage::InputRequest { params, .. } = next(&mut output) else {
            panic!()
        };
        assert_eq!(params.accepted, accept);
        if accept {
            assert_eq!(params.values["confirmed"], Value::Boolean(true));
        } else {
            assert!(params.values.is_empty());
        }
    }
}

#[test]
fn input_reserves_shared_payload_and_releases_it_on_acceptance() {
    let (_root, mut host) = host();
    let mut output = setup(&mut host, 0, &["interaction"]);
    next(&mut output);
    host.app.note_plugin_frontend(true);
    invoke(&mut host, "plugin.app-0.open");
    let api::HostMessage::Request { id, .. } = next(&mut output) else {
        panic!()
    };
    host.app
        .plugins
        .instances
        .get_mut(&0)
        .unwrap()
        .application
        .retained_payload = 48 * 1024 * 1024;
    request(
        &mut host,
        0,
        1,
        api::Request::UiConfirm {
            invocation: id.clone(),
            title: "Review".into(),
            message: "Continue?".into(),
        },
    );
    assert!(matches!(
        next(&mut output),
        api::HostMessage::Response {
            outcome: api::Response::Failure {
                error: api::Error {
                    code: api::ErrorCode::LimitExceeded,
                    ..
                }
            },
            ..
        }
    ));
    assert!(host.app.plugins.input.is_none());
    host.app
        .plugins
        .instances
        .get_mut(&0)
        .unwrap()
        .application
        .retained_payload = 0;
    request(
        &mut host,
        0,
        2,
        api::Request::UiConfirm {
            invocation: id,
            title: "Review".into(),
            message: "Continue?".into(),
        },
    );
    next(&mut output);
    assert_eq!(
        host.app.plugins.instances[&0].application.retained_payload,
        crate::plugin::interaction::SURFACE_CHARGE
    );
    key(&mut host, "Enter");
    host.sync_plugin_observers();
    assert_eq!(
        host.app.plugins.instances[&0].application.retained_payload,
        0
    );
    assert!(
        host.app.plugins.instances[&0]
            .application
            .input_surfaces
            .is_empty()
    );
}
