// SPDX-License-Identifier: MPL-2.0

use super::*;
use crate::input::KeyStroke;
use crate::plugin::application as api;

fn alias(host: &mut WorkspaceHost, owner: usize, local: &str, alias: &str) -> Result<()> {
    let mut value = command(local);
    value.alias = Some(alias.into());
    host.application_message(owner, registration(vec![value]))
}

fn type_keys(host: &mut WorkspaceHost, text: &str) {
    for ch in text.chars() {
        host.app.handle_key(KeyStroke::char(ch)).unwrap();
    }
}

#[test]
fn double_colon_completion_dispatch_help_and_backspace_use_registered_alias() {
    let (_root, mut host) = super::host();
    let mut cfg = config("configured-id");
    cfg.bindings.insert("upper".into(), "F12".into());
    let mut receiver = instance(&mut host, 0, cfg);
    alias(&mut host, 0, "upper", "time").unwrap();
    receiver.try_recv().unwrap();
    let full = host
        .app
        .parse_command("plugin.configured-id.upper")
        .unwrap();
    assert_eq!(full.id(), host.app.parse_command(":time").unwrap().id());
    assert!(host.app.parse_command("time").is_err());
    let help = crate::help::render(
        crate::help::HelpTopic::Text,
        crate::command::GrammarKind::Runyte,
        crate::keymap::BindingScope::Global,
        host.app.keymap(),
        false,
    );
    assert!(help.contains("::time"));
    assert!(
        host.app
            .keymap()
            .bindings()
            .iter()
            .any(|b| b.description.contains("::time"))
    );
    type_keys(&mut host, "::ti");
    let matches = host.app.matching_commands();
    assert!(
        matches
            .iter()
            .all(|m| matches!(m.spec.id, crate::command::CommandId::Plugin(_)))
    );
    assert_eq!(matches[0].name, ":time");
    assert_eq!(matches[0].usage(), ":time");
    assert_eq!(matches[0].other_names(), vec!["plugin.configured-id.upper"]);
    let snapshots = host.app.overlay_snapshots();
    let palette = snapshots
        .iter()
        .find(|overlay| overlay.kind == crate::snapshot::OverlayKind::CommandPalette)
        .unwrap();
    assert_eq!(palette.title, "Plugin commands");
    assert!(palette.rows[0].label.contains("::time"));
    host.app
        .handle_key(KeyStroke::parse("Tab").unwrap())
        .unwrap();
    assert_eq!(host.app.command, ":time");
    host.app
        .handle_key(KeyStroke::parse("Enter").unwrap())
        .unwrap();
    assert!(
        matches!(next(&mut receiver), api::HostMessage::Request { params, .. } if params.command == "upper")
    );
    type_keys(&mut host, "::");
    host.app
        .handle_key(KeyStroke::parse("Backspace").unwrap())
        .unwrap();
    assert!(
        host.app
            .matching_commands()
            .iter()
            .any(|m| !matches!(m.spec.id, crate::command::CommandId::Plugin(_)))
    );
}

#[test]
fn plugin_alias_does_not_shadow_builtin_and_full_spelling_stays_completable() {
    let (_root, mut host) = super::host();
    let _receiver = instance(&mut host, 0, config("case"));
    alias(&mut host, 0, "upper", "write").unwrap();
    assert_ne!(
        host.app.parse_command("write").unwrap().id(),
        host.app.parse_command(":write").unwrap().id()
    );
    host.app.command = "plugin.case.up".into();
    assert_eq!(host.app.matching_commands()[0].name, "plugin.case.upper");
    assert!(host.app.parse_command(":quit").is_err());
    host.app.command = ":".into();
    assert!(
        host.app
            .matching_commands()
            .iter()
            .all(|m| matches!(m.spec.id, crate::command::CommandId::Plugin(_)))
    );
}

#[test]
fn collisions_disable_all_claimants_keep_full_commands_and_restore_on_stop() {
    let (_root, mut host) = super::host();
    let mut cfg = config("first");
    cfg.bindings.insert("upper".into(), "F12".into());
    let _first = instance(&mut host, 0, cfg);
    alias(&mut host, 0, "upper", "time").unwrap();
    let first_id = host.app.parse_command(":time").unwrap().id();
    let _second = instance(&mut host, 1, config("second"));
    alias(&mut host, 1, "other", "time").unwrap();
    assert!(host.app.parse_command(":time").is_err());
    assert_eq!(
        host.app.parse_command("plugin.first.upper").unwrap().id(),
        first_id
    );
    assert!(host.app.parse_command("plugin.second.other").is_ok());
    assert!(host.app.plugins.alias_conflicts.contains_key("time"));
    let warning = host
        .app
        .notifications()
        .entries()
        .iter()
        .find(|entry| entry.title.contains("::time"))
        .unwrap();
    assert!(warning.body.contains(":plugin.first.upper"));
    assert!(warning.body.contains(":plugin.second.other"));
    let _third = instance(&mut host, 2, config("third"));
    alias(&mut host, 2, "another", "time").unwrap();
    assert!(host.app.notifications().entries().iter().any(
        |entry| entry.title.contains("::time") && entry.body.contains(":plugin.third.another")
    ));
    host.stop_plugin(2, "stopped by user");
    assert!(host.app.parse_command(":time").is_err());
    assert!(
        host.app
            .keymap()
            .bindings()
            .iter()
            .any(|b| b.description.contains(":plugin.first.upper"))
    );
    host.stop_plugin(1, "stopped by user");
    assert_eq!(host.app.parse_command(":time").unwrap().id(), first_id);
    assert!(
        host.app
            .keymap()
            .bindings()
            .iter()
            .any(|b| b.description.contains("::time"))
    );
    host.stop_plugin(0, "stopped by user");
    assert!(host.app.parse_command(":time").is_err());
    assert!(host.app.plugins.alias_conflicts.is_empty());
}

#[test]
fn duplicate_aliases_within_one_registration_are_disabled_and_bad_aliases_are_atomic() {
    let (_root, mut host) = super::host();
    let _receiver = instance(&mut host, 0, config("case"));
    host.application_message(
        0,
        registration(
            ["first", "second"]
                .into_iter()
                .map(|name| {
                    let mut value = command(name);
                    value.alias = Some("shared".into());
                    value
                })
                .collect(),
        ),
    )
    .unwrap();
    assert!(host.app.parse_command(":shared").is_err());
    assert!(host.app.parse_command("plugin.case.first").is_ok());
    assert!(host.app.parse_command("plugin.case.second").is_ok());
    for invalid in ["", "::time", "Time", "a b", "a.b", "é", &"x".repeat(49)] {
        let (_root, mut host) = super::host();
        let _receiver = instance(&mut host, 0, config("case"));
        assert!(alias(&mut host, 0, "upper", invalid).is_err(), "{invalid}");
        assert!(host.app.plugins.commands.is_empty());
    }
}

#[test]
fn failed_binding_registration_does_not_disable_an_existing_alias() {
    let (_root, mut host) = super::host();
    let _first = instance(&mut host, 0, config("first"));
    alias(&mut host, 0, "upper", "time").unwrap();
    let mut cfg = config("second");
    cfg.bindings.insert("upper".into(), "i".into());
    let _second = instance(&mut host, 1, cfg);
    assert!(alias(&mut host, 1, "upper", "time").is_err());
    assert!(host.app.parse_command(":time").is_ok());
    assert!(host.app.plugins.alias_conflicts.is_empty());
}

#[test]
fn stable_alias_keeps_argument_usage_completion_and_wire_command_identity() {
    let (_root, mut host) = super::host();
    let mut cfg = config("different-id");
    cfg.api = api::VERSION.to_owned();
    let mut receiver = instance(&mut host, 0, cfg);
    host.application_message(
        0,
        api::ClientMessage::Register {
            settings_schema: None,
            version: api::VERSION.into(),
            runyte: format!("={}", plugin::compatibility::HOST_VERSION),
            required_features: Default::default(),
            optional_features: Default::default(),
            name: "Timer".into(),
            commands: vec![api::Registration {
                name: "add".into(),
                alias: Some("time-add".into()),
                description: "Add a task".into(),
                context: api::CommandContext::Workspace,
                primary: false,
                arguments: serde_json::from_str(r#"[{"name":"title","type":"string"}]"#).unwrap(),
            }],
            required_capabilities: Default::default(),
            optional_capabilities: Default::default(),
        },
    )
    .unwrap();
    receiver.try_recv().unwrap();
    type_keys(&mut host, "::time-add \"猫 task\"");
    let matches = host.app.matching_commands();
    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0].usage(), ":time-add <title>");
    host.app
        .handle_key(KeyStroke::parse("Tab").unwrap())
        .unwrap();
    assert_eq!(host.app.command, ":time-add \"猫 task\"");
    host.app
        .handle_key(KeyStroke::parse("Enter").unwrap())
        .unwrap();
    assert!(
        matches!(next(&mut receiver), api::HostMessage::Request { params, .. }
        if params.command == "add" && matches!(&params.arguments["title"], plugin::arguments::Scalar::String(value) if value == "猫 task"))
    );
}
