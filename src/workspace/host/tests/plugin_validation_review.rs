// SPDX-License-Identifier: MPL-2.0

use super::interaction::{begin, key};
use super::*;
use crate::{
    input::InputEvent,
    plugin::interaction::{
        Field, FieldValidation, Kind, ValidationRequest, ValidationResult, ValidationStatus,
    },
};

fn field(secret: bool) -> Field {
    let mut field = Field::text("account", "Account".into());
    field.validate = true;
    if secret {
        field.kind = Kind::Secret;
    }
    field
}

fn start(
    host: &mut WorkspaceHost,
    output: &mut mpsc::Receiver<HostMessage>,
    secret: bool,
) -> (String, ValidationRequest) {
    begin(host, output, 1, vec![field(secret)]);
    host.app
        .handle_input(InputEvent::Text("validation-canary-🔑".into()))
        .unwrap();
    key(host, "Enter");
    host.sync_plugin_observers();
    let api::HostMessage::ValidationRequest { id, method, params } = next(output) else {
        panic!("missing validation request")
    };
    assert_eq!(method, "ui.validate");
    (id, params)
}

fn validated(params: &ValidationRequest) -> api::CommandResponse {
    api::CommandResponse::Validation {
        result: ValidationResult {
            surface: params.surface.clone(),
            revision: params.revision.clone(),
            fields: params
                .fields
                .iter()
                .map(|field| FieldValidation {
                    field: field.clone(),
                    status: ValidationStatus::Valid,
                })
                .collect(),
        },
    }
}

fn reply(host: &mut WorkspaceHost, id: String, outcome: api::CommandResponse) {
    host.handle_plugin_event(Event {
        plugin: 0,
        result: Ok(ClientMessage::Application(api::ClientMessage::Response {
            id,
            outcome,
        })),
    });
    assert!(
        host.app.plugins.instances.contains_key(&0),
        "{}",
        host.app.status
    );
}

fn application_output(output: &mut mpsc::Receiver<HostMessage>) -> Vec<api::HostMessage> {
    let mut messages = Vec::new();
    while let Ok(message) = output.try_recv() {
        if let HostMessage::Application(message) = message {
            messages.push(message);
        }
    }
    messages
}

fn assert_private(host: &mut WorkspaceHost, secret: &str) {
    assert!(!host.app.status.contains(secret));
    let geometry = crate::ui::frame_geometry(ratatui::layout::Rect::new(0, 0, 60, 18));
    let frame: crate::protocol::HostFrame = host.prepare_frame(geometry).into();
    let encoded = serde_json::to_string(&frame).unwrap();
    assert!(
        !encoded.contains(secret),
        "secret appeared in private frame"
    );
    assert!(
        host.app
            .buffers
            .iter()
            .all(|buffer| !buffer.to_string().contains(secret))
    );
    assert!(!host.app.command.contains(secret));
}

#[test]
fn validation_request_never_grants_foreground_even_before_other_input() {
    let (_root, mut host) = host();
    let mut output = setup(&mut host, 0, &["interaction"]);
    next(&mut output);
    let (id, params) = start(&mut host, &mut output, false);
    let context = &host.app.plugins.instances[&0].application.requests[&id];
    assert!(!context.foreground_allowed);
    assert!(host.app.plugin_foreground(context).is_err());
    request(
        &mut host,
        0,
        2,
        api::Request::UiConfirm {
            invocation: id.clone(),
            title: "Unauthorized follow-up".into(),
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
    assert_eq!(
        host.app.plugins.input.as_ref().unwrap().handle,
        params.surface
    );
    reply(&mut host, id, validated(&params));
    let api::HostMessage::InputRequest { id, params, .. } = next(&mut output) else {
        panic!()
    };
    assert!(params.accepted);
    assert!(
        host.app
            .plugin_foreground(&host.app.plugins.instances[&0].application.requests[&id])
            .is_ok()
    );
}

#[test]
fn cancelled_validation_cannot_submit_a_new_surface_with_matching_field_names() {
    let (_root, mut host) = host();
    let mut output = setup(&mut host, 0, &["interaction"]);
    next(&mut output);
    let (id, params) = start(&mut host, &mut output, false);
    key(&mut host, "Esc");
    host.sync_plugin_observers();
    let messages = application_output(&mut output);
    assert_eq!(
        messages
            .iter()
            .filter(|message| matches!(
                message,
                api::HostMessage::Event {
                    event: "ui.validation_cancelled",
                    ..
                }
            ))
            .count(),
        1
    );
    assert_eq!(messages.iter().filter(|message| matches!(message, api::HostMessage::InputRequest { params, .. } if !params.accepted && params.values.is_empty())).count(), 1);
    let new_surface = begin(&mut host, &mut output, 2, vec![field(false)]);
    assert_ne!(new_surface, params.surface);
    reply(&mut host, id, validated(&params));
    assert_eq!(host.app.plugins.input.as_ref().unwrap().handle, new_surface);
    assert!(
        application_output(&mut output)
            .iter()
            .all(|message| !matches!(message, api::HostMessage::InputRequest { .. }))
    );
    host.sync_plugin_observers();
    assert!(application_output(&mut output).is_empty());
}

#[test]
fn validation_timeout_releases_request_and_late_result_cannot_accept_the_surface() {
    let (_root, mut host) = host();
    let mut output = setup(&mut host, 0, &["interaction"]);
    next(&mut output);
    let (id, params) = start(&mut host, &mut output, false);
    host.app
        .plugins
        .instances
        .get_mut(&0)
        .unwrap()
        .application
        .deadlines
        .remove(&id);
    host.handle_plugin_event(Event {
        plugin: 0,
        result: Ok(ClientMessage::Deadline { token: id.clone() }),
    });
    assert!(host.app.plugins.instances.contains_key(&0));
    assert!(
        !host.app.plugins.instances[&0]
            .application
            .requests
            .contains_key(&id)
    );
    assert!(
        host.app.plugins.instances[&0]
            .application
            .validation
            .is_none()
    );
    assert!(
        host.app
            .plugins
            .input
            .as_ref()
            .unwrap()
            .validation_feedback()
            .unwrap()
            .contains("unavailable")
    );
    let messages = application_output(&mut output);
    assert_eq!(messages.iter().filter(|message| matches!(message, api::HostMessage::Event { event: "ui.validation_cancelled", data: api::EventData::ValidationCancelled(data), .. } if data.request == id && data.surface == params.surface && data.revision == params.revision)).count(), 1);
    host.handle_plugin_event(Event {
        plugin: 0,
        result: Ok(ClientMessage::Deadline { token: id.clone() }),
    });
    assert!(application_output(&mut output).is_empty());
    reply(&mut host, id, validated(&params));
    assert_eq!(
        host.app.plugins.input.as_ref().unwrap().handle,
        params.surface
    );
    assert!(
        application_output(&mut output)
            .iter()
            .all(|message| !matches!(message, api::HostMessage::InputRequest { .. }))
    );
    key(&mut host, "Enter");
    host.sync_plugin_observers();
    assert!(matches!(
        next(&mut output),
        api::HostMessage::ValidationRequest { .. }
    ));
}

#[test]
fn validation_and_secret_submit_failure_details_never_enter_snapshots() {
    let secret = "validation-canary-🔑";
    let (_root, mut host) = host();
    let mut output = setup(&mut host, 0, &["interaction"]);
    next(&mut output);
    let (id, _params) = start(&mut host, &mut output, true);
    assert_private(&mut host, secret);
    let failure = || api::CommandResponse::Failure {
        error: api::Error::new(api::ErrorCode::InvalidArgument, secret),
    };
    reply(&mut host, id, failure());
    assert_private(&mut host, secret);
    assert!(host.app.plugins.input.is_some());
    application_output(&mut output);
    key(&mut host, "Enter");
    host.sync_plugin_observers();
    let api::HostMessage::ValidationRequest { id, params, .. } = next(&mut output) else {
        panic!()
    };
    reply(&mut host, id, validated(&params));
    let api::HostMessage::InputRequest { id, params, .. } = next(&mut output) else {
        panic!()
    };
    assert!(params.accepted);
    assert!(
        serde_json::to_string(&params)
            .unwrap()
            .contains("validation-canary")
    );
    reply(&mut host, id, failure());
    assert_private(&mut host, secret);
}

#[test]
fn validation_request_quota_retains_one_intent_without_retiring_owner_or_surface() {
    let (_root, mut host) = host();
    let mut output = setup(&mut host, 0, &["interaction"]);
    next(&mut output);
    let surface = begin(&mut host, &mut output, 1, vec![field(false)]);
    host.app
        .handle_input(InputEvent::Text("account".into()))
        .unwrap();
    let context = host.app.plugins.input.as_ref().unwrap().context.clone();
    let state = &mut host.app.plugins.instances.get_mut(&0).unwrap().application;
    for n in 0..api::MAX_REQUESTS {
        state
            .requests
            .insert(format!("occupied:{n}"), context.clone());
    }
    let charge = state.retained_payload;
    key(&mut host, "Enter");
    host.sync_plugin_observers();
    let state = &host.app.plugins.instances[&0].application;
    assert_eq!(state.requests.len(), api::MAX_REQUESTS);
    assert_eq!(state.retained_payload, charge);
    assert!(state.validation.is_none());
    assert_eq!(host.app.plugins.input.as_ref().unwrap().handle, surface);
    assert!(application_output(&mut output).is_empty());
    assert!(host.app.peek_plugin_validation().is_some());
    host.app
        .plugins
        .instances
        .get_mut(&0)
        .unwrap()
        .application
        .requests
        .remove("occupied:0");
    host.sync_plugin_observers();
    assert!(matches!(
        next(&mut output),
        api::HostMessage::ValidationRequest { .. }
    ));
    assert_eq!(
        host.app.plugins.instances[&0].application.requests.len(),
        api::MAX_REQUESTS
    );
}

#[test]
fn mismatched_validation_result_retires_only_its_owner_without_exposing_values() {
    for mismatch in ["surface", "revision", "missing", "duplicate", "unknown"] {
        let (_root, mut host) = host();
        let mut output = setup(&mut host, 0, &["interaction"]);
        next(&mut output);
        let mut other = setup(&mut host, 1, &["interaction"]);
        next(&mut other);
        let (id, params) = start(&mut host, &mut output, false);
        let api::CommandResponse::Validation { mut result } = validated(&params) else {
            unreachable!()
        };
        match mismatch {
            "surface" => result.surface.push_str("-other"),
            "revision" => result.revision.push_str("-other"),
            "missing" => result.fields.clear(),
            "duplicate" => result.fields.push(result.fields[0].clone()),
            "unknown" => result.fields[0].field = "other".into(),
            _ => unreachable!(),
        }
        host.handle_plugin_event(Event {
            plugin: 0,
            result: Ok(ClientMessage::Application(api::ClientMessage::Response {
                id,
                outcome: api::CommandResponse::Validation { result },
            })),
        });
        assert!(!host.app.plugins.instances.contains_key(&0), "{mismatch}");
        assert!(host.app.plugins.instances.contains_key(&1));
        assert!(host.app.plugins.input.is_none());
        assert_private(&mut host, "validation-canary-🔑");
        assert!(application_output(&mut output).iter().all(|message| !matches!(message, api::HostMessage::InputRequest { params, .. } if params.accepted)));
    }
}
