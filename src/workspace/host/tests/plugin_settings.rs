// SPDX-License-Identifier: MPL-2.0

use super::*;
use crate::plugin::settings::{Schema, Settings};
use serde_json::json;

fn register(schema: Option<Schema>, capabilities: &[&str]) -> api::ClientMessage {
    api::ClientMessage::Register {
        settings_schema: schema,
        version: api::VERSION.into(),
        name: "Settings test".into(),
        commands: vec![api::Registration {
            arguments: vec![],
            primary: false,
            name: "run".into(),
            description: "Run".into(),
            context: api::CommandContext::Workspace,
        }],
        required_capabilities: capabilities.iter().map(|cap| (*cap).into()).collect(),
        optional_capabilities: Default::default(),
    }
}

#[test]
fn settings_schema_failure_precedes_command_capability_and_binding_side_effects() {
    for settings in [
        json!({}),
        json!({"mode":"private-config-secret"}),
        json!({"mode":"safe","private-key-secret":1}),
    ] {
        let (_root, mut host) = host();
        let mut config = config("configured-owner");
        config.api = api::Api::Epoch2;
        config.settings = Settings::from_value(settings).unwrap();
        let _receiver = instance(&mut host, 0, config);
        let schema = serde_json::from_value(
            json!({"fields":[{"name":"mode","type":"string","required":true,"enum":["safe"]}]}),
        )
        .unwrap();
        let error = host
            .application_message(0, register(Some(schema), &[]))
            .unwrap_err();
        assert!(!error.to_string().contains("private-config-secret"));
        assert!(!error.to_string().contains("private-key-secret"));
        assert!(!host.app.plugins.instances[&0].registered);
        assert!(host.app.plugins.commands.is_empty());
        assert!(
            host.app.plugins.instances[&0]
                .application
                .command_contexts
                .is_empty()
        );
        assert!(
            host.app.plugins.instances[&0]
                .application
                .capabilities
                .is_empty()
        );
    }
}

#[test]
fn settings_get_requires_capability_and_reads_only_immutable_configured_owner_values() {
    let (_root, mut host) = host();
    let mut receivers = Vec::new();
    for owner in 0..2 {
        let mut config = config(&format!("owner-{owner}"));
        config.api = api::Api::Epoch2;
        config.capabilities = vec!["settings".into()];
        config.settings =
            Settings::from_value(json!({"owner":owner,"nested":{"enabled":true}})).unwrap();
        receivers.push(instance(&mut host, owner, config));
        host.application_message(
            owner,
            register(None, if owner == 0 { &["settings"] } else { &[] }),
        )
        .unwrap();
        assert!(matches!(
            next(&mut receivers[owner]),
            api::HostMessage::Registered { .. }
        ));
    }
    // Mutating the current config object does not replace the retained instance settings.
    host.app.config.plugins.clear();
    request(&mut host, 0, 1, api::Request::SettingsGet {});
    match next(&mut receivers[0]) {
        api::HostMessage::Response {
            outcome:
                api::Response::Success {
                    result: api::ResultValue::Settings { settings },
                },
            ..
        } => assert_eq!(
            serde_json::from_str::<serde_json::Value>(settings.get()).unwrap(),
            json!({"owner":0,"nested":{"enabled":true}})
        ),
        other => panic!("unexpected {other:?}"),
    }
    request(&mut host, 1, 2, api::Request::SettingsGet {});
    assert!(matches!(
        next(&mut receivers[1]),
        api::HostMessage::Response {
            outcome: api::Response::Failure {
                error: api::Error {
                    code: api::ErrorCode::CapabilityDenied,
                    ..
                }
            },
            ..
        }
    ));
    host.app
        .plugins
        .instances
        .get_mut(&1)
        .unwrap()
        .application
        .capabilities
        .insert("settings".into());
    request(&mut host, 1, 3, api::Request::SettingsGet {});
    match next(&mut receivers[1]) {
        api::HostMessage::Response {
            outcome:
                api::Response::Success {
                    result: api::ResultValue::Settings { settings },
                },
            ..
        } => assert_eq!(
            serde_json::from_str::<serde_json::Value>(settings.get()).unwrap()["owner"],
            1
        ),
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn settings_wire_rejects_arguments_and_unknown_schema_keywords() {
    let mut absent = serde_json::to_value(register(None, &[])).unwrap();
    absent["settings_schema"] = serde_json::Value::Null;
    assert!(matches!(
        api::decode(&serde_json::to_vec(&absent).unwrap()).unwrap(),
        ClientMessage::Application(api::ClientMessage::Register {
            settings_schema: None,
            ..
        })
    ));
    for frame in [
        json!({"type":"request","id":"p:1","method":"settings.get","params":{"owner":"other"}}),
        json!({"type":"register","version":api::VERSION,"name":"Settings","commands":[],"required_capabilities":[],"optional_capabilities":[],"settings_schema":{"fields":[],"additionalProperties":true}}),
    ] {
        assert!(api::decode(&serde_json::to_vec(&frame).unwrap()).is_err());
    }
}

#[test]
fn settings_schema_is_optional_on_wire_and_validates_even_without_settings_grant() {
    let (_root, mut host) = host();
    let mut config = config("typed-owner");
    config.api = api::Api::Epoch2;
    config.settings = Settings::from_value(json!({"enabled":true})).unwrap();
    let mut receiver = instance(&mut host, 0, config);
    let schema = serde_json::from_value(
        json!({"fields":[{"name":"enabled","type":"boolean","required":true}]}),
    )
    .unwrap();
    let frame = serde_json::to_vec(&register(Some(schema), &[])).unwrap();
    let ClientMessage::Application(message) = api::decode(&frame).unwrap() else {
        panic!("not application frame")
    };
    host.application_message(0, message).unwrap();
    assert!(matches!(
        next(&mut receiver),
        api::HostMessage::Registered { .. }
    ));
    assert!(host.app.plugins.instances[&0].registered);
    request(&mut host, 0, 1, api::Request::SettingsGet {});
    assert!(matches!(
        next(&mut receiver),
        api::HostMessage::Response {
            outcome: api::Response::Failure {
                error: api::Error {
                    code: api::ErrorCode::CapabilityDenied,
                    ..
                }
            },
            ..
        }
    ));
}

#[test]
fn queued_settings_reads_share_canonical_payload_without_retaining_parsed_copies() {
    let (_root, mut host) = host();
    let settings =
        Settings::from_value(json!({"rows":(0..1024).map(|n|json!({"id":n})).collect::<Vec<_>>()}))
            .unwrap();
    let expected = settings.encoded();
    let mut config = config("shared-settings");
    config.api = api::Api::Epoch2;
    config.settings = settings;
    config.capabilities = vec!["settings".into()];
    drop(instance(&mut host, 0, config));
    // Match the production output capacity; the shared fixture has only eight slots.
    let (sender, mut receiver) = mpsc::channel(32);
    host.app.plugins.instances.get_mut(&0).unwrap().sender = plugin::Sender::new(sender);
    host.application_message(0, register(None, &["settings"]))
        .unwrap();
    assert!(matches!(
        next(&mut receiver),
        api::HostMessage::Registered { .. }
    ));
    for id in 1..=16 {
        request(&mut host, 0, id, api::Request::SettingsGet {});
    }
    for _ in 1..=16 {
        match next(&mut receiver) {
            api::HostMessage::Response {
                outcome:
                    api::Response::Success {
                        result: api::ResultValue::Settings { settings },
                    },
                ..
            } => assert!(std::sync::Arc::ptr_eq(&expected, &settings)),
            other => panic!("unexpected {other:?}"),
        }
    }
}
