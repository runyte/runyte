// SPDX-License-Identifier: MPL-2.0
use super::interaction::{begin, key};
use super::*;
use crate::{
    input::InputEvent,
    plugin::interaction::{Field, Kind},
};
use serde_json::{Value, json};

fn setup_validation(host: &mut WorkspaceHost) -> mpsc::Receiver<HostMessage> {
    let mut initial = setup(host, 0, &["interaction"]);
    next(&mut initial);
    let (sender, output) = mpsc::channel(32);
    host.app.plugins.instances.get_mut(&0).unwrap().sender = plugin::Sender::new(sender);
    output
}
fn checked(id: &str, secret: bool) -> Field {
    let mut field = Field::text(id, id.into());
    field.validate = true;
    field.validation_message = Some("Choose another value".into());
    if secret {
        field.kind = Kind::Secret;
    }
    field
}
fn type_text(host: &mut WorkspaceHost, text: &str) {
    host.app
        .handle_input(InputEvent::Text(text.into()))
        .unwrap();
}
fn validation(output: &mut mpsc::Receiver<HostMessage>) -> Value {
    loop {
        let message = serde_json::to_value(next(output)).unwrap();
        if message["event"] == "ui.validation_cancelled" {
            continue;
        }
        assert_eq!(message["method"], "ui.validate", "{message}");
        return message;
    }
}
fn answer(host: &mut WorkspaceHost, request: &Value, status: &str) {
    let result = json!({"kind": "validation", "surface": request["params"]["surface"],
        "revision": request["params"]["revision"], "fields": request["params"]["fields"].as_array().unwrap().iter()
            .map(|field| json!({"field": field, "status": status})).collect::<Vec<_>>()});
    host.application_message(
        0,
        serde_json::from_value(json!({"type": "response", "id": request["id"], "result": result}))
            .unwrap(),
    )
    .unwrap();
    host.sync_plugin_observers();
}
fn accepted(output: &mut mpsc::Receiver<HostMessage>) -> Value {
    let message = serde_json::to_value(next(output)).unwrap();
    assert_eq!(message["method"], "ui.submit", "{message}");
    assert_eq!(message["params"]["accepted"], true);
    message
}

#[test]
fn validation_discloses_only_opted_in_secrets_on_enter_then_submits_all_fields() {
    let (_root, mut host) = host();
    let mut output = setup_validation(&mut host);
    let mut final_only = Field::text("token", "Token".into());
    final_only.kind = Kind::Secret;
    begin(
        &mut host,
        &mut output,
        1,
        vec![
            checked("name", false),
            checked("password", true),
            final_only,
        ],
    );
    type_text(&mut host, "example");
    key(&mut host, "Tab");
    type_text(&mut host, "validation-secret");
    key(&mut host, "Tab");
    type_text(&mut host, "final-only-secret");
    host.sync_plugin_observers();
    assert!(
        output.try_recv().is_err(),
        "Typing must not send validation values"
    );
    key(&mut host, "Enter");
    host.sync_plugin_observers();
    let request = validation(&mut output);
    assert_eq!(request["params"]["fields"], json!(["name", "password"]));
    assert_eq!(
        request["params"]["values"],
        json!({"name": "example", "password": "validation-secret"})
    );
    let geometry = crate::ui::frame_geometry(ratatui::layout::Rect::new(0, 0, 80, 24));
    let frame: crate::protocol::HostFrame = host.prepare_frame(geometry).into();
    let encoded = serde_json::to_string(&frame).unwrap();
    assert!(!encoded.contains("validation-secret") && !encoded.contains("final-only-secret"));
    answer(&mut host, &request, "valid");
    let submission = accepted(&mut output);
    assert_eq!(submission["params"]["values"]["token"], "final-only-secret");
    assert!(!host.app.plugin_input_active());
}

#[test]
fn editing_during_validation_rejects_stale_success_and_validates_new_revision() {
    let (_root, mut host) = host();
    let mut output = setup_validation(&mut host);
    begin(&mut host, &mut output, 1, vec![checked("name", false)]);
    type_text(&mut host, "first");
    key(&mut host, "Enter");
    host.sync_plugin_observers();
    let old = validation(&mut output);
    type_text(&mut host, "-next");
    answer(&mut host, &old, "valid");
    assert!(host.app.plugin_input_active());
    while let Ok(message) = output.try_recv() {
        assert!(!matches!(
            message,
            HostMessage::Application(api::HostMessage::InputRequest { .. })
        ));
    }
    key(&mut host, "Enter");
    host.sync_plugin_observers();
    let new = validation(&mut output);
    assert_ne!(new["params"]["revision"], old["params"]["revision"]);
    assert_eq!(new["params"]["values"]["name"], "first-next");
    answer(&mut host, &new, "valid");
    assert_eq!(
        accepted(&mut output)["params"]["values"]["name"],
        "first-next"
    );
}

#[test]
fn repeated_enter_keeps_one_flight_and_only_latest_intent_can_submit() {
    let (_root, mut host) = host();
    let mut output = setup_validation(&mut host);
    begin(&mut host, &mut output, 1, vec![checked("name", false)]);
    type_text(&mut host, "example");
    key(&mut host, "Enter");
    host.sync_plugin_observers();
    let old = validation(&mut output);
    key(&mut host, "Enter");
    key(&mut host, "Enter");
    host.sync_plugin_observers();
    while let Ok(message) = output.try_recv() {
        let message = match message {
            HostMessage::Application(value) => serde_json::to_value(value).unwrap(),
            HostMessage::Deadline { .. } => continue,
            other => panic!("{other:?}"),
        };
        assert_eq!(message["event"], "ui.validation_cancelled", "{message}");
    }
    answer(&mut host, &old, "valid");
    assert!(host.app.plugin_input_active());
    let latest = validation(&mut output);
    assert_ne!(latest["id"], old["id"]);
    answer(&mut host, &latest, "valid");
    accepted(&mut output);
}
