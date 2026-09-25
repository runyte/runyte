// SPDX-License-Identifier: MPL-2.0

use super::*;
use crate::{
    app::{App, CommandOutcome, HostPorts},
    clipboard::SystemClipboard,
    command::{CommandExecutionContext, CommandInvocation, EditorCommand},
    plugin::{PluginConfig, application as api},
    selection::{Range, Selection},
    test_support::TestRuntimeRoot,
    text::Transaction,
    workspace::{BufferId, BufferRevision},
};
use tokio::sync::mpsc;

struct Clipboard;
impl SystemClipboard for Clipboard {
    fn read(&mut self) -> Result<String> {
        anyhow::bail!("inert")
    }
    fn write(&mut self, _: &str) -> Result<()> {
        Ok(())
    }
}

fn host() -> (TestRuntimeRoot, WorkspaceHost) {
    let root = TestRuntimeRoot::new("plugins").unwrap();
    let app = App::new_in_isolated_project(root.path(), HostPorts::isolated(Box::new(Clipboard)))
        .unwrap();
    (root, WorkspaceHost::new(app))
}
fn config(id: &str) -> PluginConfig {
    PluginConfig {
        settings: Default::default(),
        id: id.into(),
        api: api::VERSION.to_owned(),
        runyte: format!("={}", plugin::compatibility::HOST_VERSION),
        capabilities: vec!["text".into(), "selections".into(), "workspace".into()],
        enabled: true,
        executable: "/nonexistent/runyte-plugin".into(),
        args: vec![],
        bindings: Default::default(),
    }
}
fn instance(
    host: &mut WorkspaceHost,
    id: usize,
    config: PluginConfig,
) -> mpsc::Receiver<HostMessage> {
    let (sender, receiver) = mpsc::channel(8);
    host.app.plugins.instances.insert(
        id,
        Instance {
            config,
            sender: plugin::Sender::new(sender),
            registered: false,
            application: Default::default(),
        },
    );
    receiver
}
fn registration(commands: Vec<api::Registration>) -> api::ClientMessage {
    api::ClientMessage::Register {
        version: api::VERSION.into(),
        runyte: format!("={}", plugin::compatibility::HOST_VERSION),
        settings_schema: None,
        name: "Test plugin".into(),
        help_topics: None,
        commands,
        required_capabilities: Default::default(),
        optional_capabilities: ["text".into(), "selections".into(), "workspace".into()].into(),
        required_features: Default::default(),
        optional_features: Default::default(),
    }
}
fn command(name: &str) -> api::Registration {
    api::Registration {
        default_binding: None,
        presentation: None,
        alias: None,
        arguments: vec![],
        primary: false,
        name: name.into(),
        description: "Uppercase selections".into(),
        context: api::CommandContext::Buffer,
    }
}
fn register(host: &mut WorkspaceHost, id: usize) -> Result<()> {
    host.application_message(id, registration(vec![command("upper")]))
}
fn next(receiver: &mut mpsc::Receiver<HostMessage>) -> api::HostMessage {
    loop {
        match receiver.try_recv().unwrap() {
            HostMessage::Application(message) => return message,
            HostMessage::Deadline { .. } => {}
        }
    }
}
fn call(
    host: &mut WorkspaceHost,
    receiver: &mut mpsc::Receiver<HostMessage>,
    n: u64,
    method: &str,
    params: serde_json::Value,
) -> serde_json::Value {
    host.handle_plugin_event(Event {
        plugin: 0,
        result: Ok(ClientMessage::Application(api::ClientMessage::Request {
            id: format!("p:{n}"),
            request: serde_json::from_value(serde_json::json!({"method":method,"params":params}))
                .unwrap(),
        })),
    });
    serde_json::to_value(next(receiver)).unwrap()
}
fn capture(
    host: &mut WorkspaceHost,
    receiver: &mut mpsc::Receiver<HostMessage>,
) -> api::Invocation {
    invoke(host, "plugin.case.upper");
    let api::HostMessage::Request { params, .. } = next(receiver) else {
        panic!("missing invocation")
    };
    params
}
fn seed(host: &mut WorkspaceHost, text: &str) {
    host.apply_expected_transaction(
        BufferId::from_index(0),
        BufferRevision::from_raw(host.app.buffers[0].revision()),
        Transaction::insert(0, text),
    )
    .unwrap();
    host.app.panes.get_mut(&0).unwrap().selection = Selection::point(0);
}
fn invoke(host: &mut WorkspaceHost, name: &str) -> CommandOutcome {
    let command = host.app.parse_command(name).unwrap();
    host.app.execute(command).unwrap()
}
fn undo(host: &mut WorkspaceHost) {
    host.app
        .execute(
            CommandInvocation::editor(EditorCommand::Undo, CommandExecutionContext::default())
                .unwrap(),
        )
        .unwrap();
}

#[test]
fn registry_colon_binding_help_hints_collisions_cleanup_and_stop() {
    let (_root, mut host) = host();
    let mut cfg = config("case");
    cfg.bindings.insert("upper".into(), "F12".into());
    let mut receiver = instance(&mut host, 0, cfg);
    register(&mut host, 0).unwrap();
    receiver.try_recv().unwrap();
    let invocation = host.app.parse_command("plugin.case.upper").unwrap();
    assert!(
        host.app
            .matching_commands()
            .iter()
            .any(|m| m.name == "plugin.case.upper" && m.spec.description == "Uppercase selections")
    );
    let key = crate::input::KeyStroke::parse("F12").unwrap();
    host.app
        .handle_input(crate::input::InputEvent::Key(key))
        .unwrap();
    assert!(matches!(
        receiver.try_recv().unwrap(),
        HostMessage::Application(api::HostMessage::Request { .. })
    ));
    // Stable applications may accept concurrent commands within their limit.
    assert!(matches!(
        invoke(&mut host, "plugin.case.upper"),
        CommandOutcome::AsynchronousRequest(_)
    ));
    let binding = host
        .app
        .keymap()
        .bindings()
        .iter()
        .find(|b| b.target.id() == invocation.id())
        .unwrap();
    assert!(binding.description.contains("plugin.case.upper"));
    let help = crate::help::render(
        crate::help::HelpTopic::Text,
        crate::command::GrammarKind::Runyte,
        crate::keymap::BindingScope::Global,
        host.app.keymap(),
        false,
    );
    assert!(help.contains("plugin.case.upper"));
    let mut collision = config("other");
    collision.bindings.insert("upper".into(), "F12".into());
    let _other = instance(&mut host, 1, collision);
    assert!(register(&mut host, 1).is_err());
    assert!(host.app.parse_command("plugin.other.upper").is_err());
    assert!(register(&mut host, 0).is_err());
    invoke(&mut host, "plugin.case.stop");
    host.sync_plugin_observers();
    assert!(host.app.parse_command("plugin.case.upper").is_err());
    assert!(
        host.app
            .keymap()
            .bindings()
            .iter()
            .all(|b| b.target.id() != invocation.id())
    );
    assert!(matches!(
        host.app.execute(invocation).unwrap(),
        CommandOutcome::UserError(_)
    ));
}

#[tokio::test]
async fn disabled_spawn_failure_and_host_attachment_do_not_duplicate_instances() {
    let (_root, mut host) = host();
    let mut disabled = config("off");
    disabled.enabled = false;
    host.app.config.plugins.push(disabled);
    // The queue exists whatever is enabled, so a configuration reload can
    // start a plugin this launch left disabled. Nothing is spawned for it.
    assert!(host.start_plugins().is_some());
    assert!(host.plugin_workers.is_empty());
    let (_root, mut host) = self::host();
    host.app.config.plugins.push(config("missing"));
    let mut events = host.start_plugins().unwrap();
    host.app.note_frontend_attached();
    host.app.note_frontend_attached();
    assert!(host.start_plugins().is_none());
    assert_eq!(host.plugin_workers.len(), 1);
    let event = tokio::time::timeout(std::time::Duration::from_secs(3), events.recv())
        .await
        .unwrap()
        .unwrap();
    host.handle_plugin_event(event);
    assert!(host.plugin_workers.is_empty());
    assert!(host.app.plugins.instances.is_empty());
    assert!(host.app.status.contains("could not start"));
}

#[tokio::test]
async fn process_malformed_output_exit_and_deadline_remove_commands() {
    for (behavior, expected) in [
        (
            "read -r hello\nprintf 'invalid-json\\n'\nwhile read -r line; do :; done\n",
            "protocol or IO failed",
        ),
        ("exit 7\n", "Plugin"),
        (
            "read -r hello\nwhile read -r line; do :; done\n",
            "timed out",
        ),
    ] {
        let (root, mut host) = host();
        let program = root.join("worker");
        std::os::unix::fs::symlink(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/fixtures/stand-in"),
            &program,
        )
        .unwrap();
        std::fs::write(root.join("worker.behavior"), behavior).unwrap();
        let mut cfg = config("worker");
        cfg.executable = program;
        host.app.config.plugins.push(cfg);
        let mut events = host.start_plugins().unwrap();
        let event = tokio::time::timeout(
            plugin::TIMEOUT + std::time::Duration::from_secs(3),
            events.recv(),
        )
        .await
        .unwrap()
        .unwrap();
        host.handle_plugin_event(event);
        assert!(host.app.plugins.instances.is_empty(), "{expected}");
        assert!(host.app.status.contains(expected), "{}", host.app.status);
    }
}

#[test]
fn prefixed_plugin_binding_hints_use_the_live_registration_and_cleanup() {
    let (_root, mut host) = host();
    let mut cfg = config("case");
    cfg.bindings.insert("upper".into(), "F12 a".into());
    let mut receiver = instance(&mut host, 0, cfg);
    register(&mut host, 0).unwrap();
    receiver.try_recv().unwrap();
    let mut hints = crate::key_hints::KeyHintState::default();
    let prefix = crate::input::KeyStroke::parse("F12").unwrap();
    hints.observe(prefix, crate::command::Mode::Normal, host.app.keymap());
    let rows = hints.rows(host.app.keymap(), crate::command::Mode::Normal);
    assert_eq!(rows.len(), 1);
    assert_eq!(
        rows[0].target.unwrap().id(),
        host.app.parse_command("plugin.case.upper").unwrap().id()
    );
    assert!(crate::key_hints::key_hint_description(&rows[0]).contains("plugin.case.upper"));
    let stale = host.app.parse_command("plugin.case.upper").unwrap();
    host.stop_plugin(0, "stopped by user");
    assert!(
        hints
            .rows(host.app.keymap(), crate::command::Mode::Normal)
            .is_empty()
    );
    let mut next = instance(&mut host, 0, config("next"));
    register(&mut host, 0).unwrap();
    next.try_recv().unwrap();
    assert_ne!(
        host.app.parse_command("plugin.next.upper").unwrap().id(),
        stale.id()
    );
    assert!(matches!(
        host.app.execute(stale).unwrap(),
        CommandOutcome::UserError(_)
    ));
    assert!(next.try_recv().is_err());
}

#[test]
fn plugin_bindings_cannot_claim_grammar_counts_or_prefix_cancellation() {
    for sequence in ["1 a", "F12 Escape", "F12 Backspace", "Space"] {
        let (_root, mut host) = host();
        let mut cfg = config("case");
        cfg.bindings.insert("upper".into(), sequence.into());
        let _receiver = instance(&mut host, 0, cfg);
        assert!(register(&mut host, 0).is_err(), "{sequence}");
        assert!(host.app.parse_command("plugin.case.upper").is_err());
    }
}

#[test]
fn unicode_multiselection_capture_focus_mapping_and_single_undo_without_frame() {
    use serde_json::json;
    let (root, mut host) = host();
    seed(&mut host, "éß 😀xy");
    let mut receiver = instance(&mut host, 0, config("case"));
    register(&mut host, 0).unwrap();
    next(&mut receiver);
    host.app.panes.get_mut(&0).unwrap().selection =
        Selection::new(vec![Range::new(1, 0), Range::new(3, 5)], 1);
    let params = capture(&mut host, &mut receiver);
    let selected = call(
        &mut host,
        &mut receiver,
        1,
        "selection.get",
        json!({"pane":params.pane}),
    );
    assert_eq!(selected["result"]["primary"], 1);
    assert_eq!(
        selected["result"]["ranges"],
        json!([{"anchor":1,"head":0},{"anchor":3,"head":5}])
    );
    assert_eq!(
        selected["result"]["spans"],
        json!([{"from":0,"to":2},{"from":3,"to":6}])
    );
    let read = call(
        &mut host,
        &mut receiver,
        2,
        "buffer.read",
        json!({"buffer":params.buffer,"expected_revision":params.buffer_revision,"from":0,"to":6}),
    );
    assert_eq!(read["result"]["text"], "éß 😀xy");
    assert!(host.current_frame_id().is_none());
    host.app
        .execute(crate::command::parse_colon_command("vsplit").unwrap())
        .unwrap();
    let path = root.join("other.txt");
    std::fs::write(&path, "other").unwrap();
    let other = host.open_buffer(path, true).unwrap();
    let edited = call(
        &mut host,
        &mut receiver,
        3,
        "buffer.edit",
        json!({"buffer":params.buffer,"expected_revision":params.buffer_revision,"changes":[{"from":0,"to":2,"text":"ÉSS"},{"from":3,"to":6,"text":"😀XY"}]}),
    );
    assert!(edited.get("error").is_none(), "{edited}");
    assert_eq!(host.app.buffers[0].to_string(), "ÉSS 😀XY");
    assert_eq!(BufferId::from_index(host.app.active().buffer), other);
    host.app.active_pane = 0;
    undo(&mut host);
    assert_eq!(host.app.buffers[0].to_string(), "éß 😀xy");
    undo(&mut host);
    assert_eq!(host.app.buffers[0].to_string(), "");
}

#[test]
fn stale_closed_readonly_and_invalid_edits_preserve_text_revision_and_undo_group() {
    use serde_json::json;
    for scenario in [
        "edit", "undo", "closed", "readonly", "overlap", "range", "noop",
    ] {
        let (_root, mut host) = host();
        seed(&mut host, "abc");
        host.app.buffers[0].begin_undo_group();
        let revision = host.app.buffers[0].revision();
        host.apply_expected_transaction(
            BufferId::from_index(0),
            BufferRevision::from_raw(revision),
            Transaction::insert(3, "!"),
        )
        .unwrap();
        let mut receiver = instance(&mut host, 0, config("case"));
        register(&mut host, 0).unwrap();
        next(&mut receiver);
        let params = capture(&mut host, &mut receiver);
        let kind = host.app.buffers[0].kind.clone();
        match scenario {
            "edit" | "undo" => {
                let revision = host.app.buffers[0].revision();
                host.apply_expected_transaction(
                    BufferId::from_index(0),
                    BufferRevision::from_raw(revision),
                    Transaction::insert(4, "+"),
                )
                .unwrap();
                if scenario == "undo" {
                    undo(&mut host);
                }
            }
            "closed" => host.close_buffer(BufferId::from_index(0), true).unwrap(),
            "readonly" => host.app.buffers[0].kind = crate::buffer::BufferKind::Help,
            _ => {}
        }
        let before = host.app.buffers[0].to_string();
        let revision = host.app.buffers[0].revision();
        let history = host.app.buffers[0].history_len();
        let changes = match scenario {
            "overlap" => json!([{"from":0,"to":2,"text":"X"},{"from":1,"to":3,"text":"Y"}]),
            "range" => json!([{"from":0,"to":100,"text":"X"}]),
            "noop" => json!([]),
            _ => json!([{"from":0,"to":1,"text":"A"}]),
        };
        let reply = call(
            &mut host,
            &mut receiver,
            1,
            "buffer.edit",
            json!({"buffer":params.buffer,"expected_revision":params.buffer_revision,"changes":changes}),
        );
        if scenario == "noop" {
            assert!(reply.get("error").is_none(), "{reply}");
        } else {
            let code = match scenario {
                "edit" | "undo" => "stale",
                "closed" => "closed",
                "readonly" => "read_only",
                _ => "invalid_argument",
            };
            assert_eq!(reply["error"]["code"], code, "{scenario}: {reply}");
        }
        assert_eq!(host.app.buffers[0].to_string(), before, "{scenario}");
        assert_eq!(host.app.buffers[0].revision(), revision, "{scenario}");
        assert_eq!(host.app.buffers[0].history_len(), history, "{scenario}");
        host.app.buffers[0].kind = kind;
        if matches!(scenario, "readonly" | "overlap" | "range" | "noop") {
            host.apply_expected_transaction(
                BufferId::from_index(0),
                BufferRevision::from_raw(revision),
                Transaction::insert(before.chars().count(), "?"),
            )
            .unwrap();
            undo(&mut host);
            assert_eq!(host.app.buffers[0].to_string(), "abc", "{scenario}");
        }
    }
}

#[test]
fn plugin_edit_splits_an_existing_insert_undo_group() {
    use serde_json::json;
    let (_root, mut host) = host();
    seed(&mut host, "abc");
    host.app.buffers[0].begin_undo_group();
    let revision = host.app.buffers[0].revision();
    host.apply_expected_transaction(
        BufferId::from_index(0),
        BufferRevision::from_raw(revision),
        Transaction::insert(3, "!"),
    )
    .unwrap();
    let mut receiver = instance(&mut host, 0, config("case"));
    register(&mut host, 0).unwrap();
    next(&mut receiver);
    let params = capture(&mut host, &mut receiver);
    let reply = call(
        &mut host,
        &mut receiver,
        1,
        "buffer.edit",
        json!({"buffer":params.buffer,"expected_revision":params.buffer_revision,"changes":[{"from":0,"to":1,"text":"A"}]}),
    );
    assert!(reply.get("error").is_none(), "{reply}");
    assert_eq!(host.app.buffers[0].to_string(), "Abc!");
    undo(&mut host);
    assert_eq!(host.app.buffers[0].to_string(), "abc!");
    undo(&mut host);
    assert_eq!(host.app.buffers[0].to_string(), "abc");
}

#[test]
fn subscriptions_order_coalescing_unsubscribe_and_closed_buffers_use_stable_events() {
    use serde_json::json;
    let (_root, mut host) = host();
    seed(&mut host, "abc");
    let mut receiver = instance(&mut host, 0, config("case"));
    register(&mut host, 0).unwrap();
    next(&mut receiver);
    let (sender, replacement) = mpsc::channel(32);
    host.app.plugins.instances.get_mut(&0).unwrap().sender = plugin::Sender::new(sender);
    receiver = replacement;
    let initial = call(
        &mut host,
        &mut receiver,
        1,
        "event.subscribe",
        json!({"sources":[{"kind":"buffers"}]}),
    );
    let baseline = &initial["result"];
    let buffer = baseline["sources"][0]["source"]["buffer"].clone();
    let revision = baseline["sources"][0]["state"]["revision"].clone();
    let response = call(
        &mut host,
        &mut receiver,
        2,
        "buffer.edit",
        json!({"buffer":buffer,"expected_revision":revision,"changes":[{"from":0,"to":1,"text":"A"}]}),
    );
    assert!(response.get("error").is_none(), "{response}");
    host.sync_plugin_observers();
    let changed = serde_json::to_value(next(&mut receiver)).unwrap();
    assert_eq!(changed["event"], "event.changed");
    assert_eq!(changed["data"]["subscription"], baseline["subscription"]);
    host.sync_plugin_observers();
    assert!(receiver.try_recv().is_err());
    undo(&mut host);
    host.sync_plugin_observers();
    assert!(matches!(
        next(&mut receiver),
        api::HostMessage::Event {
            event: "event.changed",
            ..
        }
    ));
    let removed = call(
        &mut host,
        &mut receiver,
        3,
        "event.unsubscribe",
        json!({"subscription":baseline["subscription"]}),
    );
    assert!(removed.get("error").is_none());
    undo(&mut host);
    host.sync_plugin_observers();
    assert!(receiver.try_recv().is_err());
    let renewed = call(
        &mut host,
        &mut receiver,
        4,
        "event.subscribe",
        json!({"sources":[{"kind":"buffers"}]}),
    );
    assert!(renewed.get("error").is_none());
    host.close_buffer(BufferId::from_index(0), true).unwrap();
    host.sync_plugin_observers();
    let closed = serde_json::to_value(next(&mut receiver)).unwrap();
    assert_eq!(closed["event"], "event.closed");
    assert_eq!(closed["data"]["sources"][0]["state"]["kind"], "closed");
}

#[test]
fn failed_registration_leaves_all_negotiated_state_and_registry_unchanged() {
    for failure in [
        "protocol",
        "range",
        "feature",
        "capability",
        "capability_overlap",
        "name",
        "description",
        "binding",
        "queue",
    ] {
        let (_root, mut host) = host();
        let mut cfg = config("case");
        if failure == "binding" {
            cfg.bindings.insert("upper".into(), "i".into());
        }
        let mut receiver = Some(instance(&mut host, 0, cfg));
        let mut registration = registration(vec![command("upper")]);
        let api::ClientMessage::Register {
            version,
            runyte,
            required_features,
            required_capabilities,
            commands,
            ..
        } = &mut registration
        else {
            unreachable!()
        };
        match failure {
            "protocol" => *version = "runyte-experimental-2".into(),
            "range" => *runyte = "=99.0.0".into(),
            "feature" => {
                required_features.insert("future".into());
            }
            "capability" => {
                required_capabilities.insert("jobs".into());
            }
            "capability_overlap" => {
                required_capabilities.insert("text".into());
            }
            "name" => commands[0].name = "stop".into(),
            "description" => commands[0].description = "bad\nlabel".into(),
            _ => {}
        }
        if failure == "queue" {
            drop(receiver.take());
        }
        let before = host.app.plugins.next_command;
        assert!(
            host.application_message(0, registration).is_err(),
            "{failure}"
        );
        assert_eq!(host.app.plugins.next_command, before, "{failure}");
        assert!(host.app.plugins.commands.is_empty());
        let owner = &host.app.plugins.instances[&0];
        assert!(!owner.registered);
        assert!(owner.application.capabilities.is_empty());
        assert!(owner.application.features.is_empty());
        assert!(owner.application.command_contexts.is_empty());
    }
}

#[test]
fn retired_owner_rejects_queued_mutations_and_does_not_replay_commands() {
    use serde_json::json;
    let (_root, mut host) = host();
    seed(&mut host, "abc");
    let mut receiver = instance(&mut host, 0, config("case"));
    register(&mut host, 0).unwrap();
    next(&mut receiver);
    let params = capture(&mut host, &mut receiver);
    invoke(&mut host, "plugin.case.stop");
    host.handle_plugin_event(Event{plugin:0,result:Ok(ClientMessage::Application(api::ClientMessage::Request{id:"p:late".into(),request:serde_json::from_value(json!({"method":"buffer.edit","params":{"buffer":params.buffer,"expected_revision":params.buffer_revision,"changes":[{"from":0,"to":1,"text":"X"}]}})).unwrap()}))});
    assert_eq!(host.app.buffers[0].to_string(), "abc");
    assert!(host.app.plugins.instances.is_empty());
    assert!(host.app.plugins.commands.is_empty());
}

#[path = "plugin_applications.rs"]
mod applications;

#[path = "plugin_aliases.rs"]
mod aliases;

#[path = "plugin_configuration_reload.rs"]
mod configuration_reload;
#[path = "plugin_manager.rs"]
mod manager;
#[path = "plugin_manager_review.rs"]
mod manager_review;

#[test]
fn asynchronous_plugin_completion_updates_only_its_own_action_echo() {
    for intervening_key in [false, true] {
        let (_root, mut host) = host();
        seed(&mut host, "abc");
        let mut cfg = config("case");
        cfg.bindings.insert("upper".into(), "F12".into());
        let mut receiver = instance(&mut host, 0, cfg);
        register(&mut host, 0).unwrap();
        next(&mut receiver);
        host.app
            .handle_input(crate::input::InputEvent::Key(
                crate::input::KeyStroke::parse("F12").unwrap(),
            ))
            .unwrap();
        let api::HostMessage::Request { id, .. } = next(&mut receiver) else {
            panic!()
        };
        assert!(host.app.displayed_status_message().contains("accepted"));
        if intervening_key {
            host.app
                .handle_input(crate::input::InputEvent::Key(
                    crate::input::KeyStroke::char('l'),
                ))
                .unwrap();
        }
        let before = host.app.displayed_status_message().to_owned();
        host.handle_plugin_event(Event {
            plugin: 0,
            result: Ok(ClientMessage::Application(api::ClientMessage::Response {
                id,
                outcome: api::CommandResponse::Success {
                    result: api::CommandResult { job: None },
                },
            })),
        });
        if intervening_key {
            assert_eq!(host.app.displayed_status_message(), before);
        } else {
            assert!(host.app.displayed_status_message().contains("completed"));
        }
        assert!(
            host.app.plugins.instances[&0]
                .application
                .requests
                .is_empty()
        );
    }
}

#[tokio::test]
async fn real_process_transforms_text_and_preserves_one_host_instance() {
    let (root, mut host) = host();
    seed(&mut host, "éß");
    let program = root.join("case");
    std::os::unix::fs::symlink(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/fixtures/stand-in"),
        &program,
    )
    .unwrap();
    let registration = serde_json::to_string(&registration(vec![command("upper")])).unwrap();
    let behavior = r#"
printf 'started\n' >> "$0.starts"
read -r hello
printf '%s\n' 'REGISTRATION'
read -r registered
while read -r request; do
    case "$request" in
        *'"method":"command.invoke"'*)
            buffer=${request#*\"buffer\":\"}; buffer=${buffer%%\"*}
            revision=${request#*\"buffer_revision\":\"}; revision=${revision%%\"*}
            printf '{"type":"request","id":"p:1","method":"buffer.edit","params":{"buffer":"%s","expected_revision":"%s","changes":[{"from":0,"to":2,"text":"ÉSS"}]}}\n' "$buffer" "$revision"
            ;;
    esac
done
"#.replace("REGISTRATION", &registration);
    std::fs::write(root.join("case.behavior"), behavior).unwrap();
    let mut cfg = config("case");
    cfg.executable = program;
    host.app.config.plugins.push(cfg);
    let mut events = host.start_plugins().unwrap();
    manager::until(&mut host, &mut events, |host| {
        host.app
            .plugins
            .instances
            .values()
            .any(|instance| instance.registered)
    })
    .await;
    invoke(&mut host, "plugin.case.upper");
    host.app.note_frontend_attached();
    assert!(host.start_plugins().is_none());
    manager::until(&mut host, &mut events, |host| {
        host.app.buffers[0].to_string() == "ÉSS"
    })
    .await;
    assert_eq!(
        std::fs::read_to_string(root.join("case.starts")).unwrap(),
        "started\n"
    );
    undo(&mut host);
    assert_eq!(host.app.buffers[0].to_string(), "éß");
    host.shutdown_plugins().await.unwrap();
}

#[test]
fn command_presentation_registration_is_negotiated_bounded_atomic_and_charged() {
    use crate::plugin::presentation::Presentation;
    let (_root, mut host) = host();
    let _receiver = instance(&mut host, 0, config("labels"));
    let mut cmd = command("upper");
    cmd.presentation = Some(Presentation {
        label: "Uppercase text".into(),
        group: Some("Edit".into()),
        order: 0,
        listed: true,
    });
    let register = |host: &mut WorkspaceHost, commands, features| {
        host.register_plugin_commands(
            0,
            commands,
            Default::default(),
            features,
            format!("={}", plugin::compatibility::HOST_VERSION),
        )
    };
    let features = || [api::VIEW_ACTION_PRESENTATION.to_owned()].into();
    let mut wire = serde_json::to_value(&cmd).unwrap();
    wire["presentation"] = serde_json::Value::Null;
    assert!(serde_json::from_value::<api::Registration>(wire).is_err());
    assert!(register(&mut host, vec![cmd.clone()], Default::default()).is_err());
    let mut invalid = cmd.clone();
    invalid.presentation.as_mut().unwrap().label.push('\n');
    assert!(register(&mut host, vec![invalid], features()).is_err());
    let many = (0..17)
        .map(|i| {
            let mut c = cmd.clone();
            c.name = format!("c{i}");
            c.presentation.as_mut().unwrap().group = Some(format!("g{i}"));
            c
        })
        .collect();
    assert!(register(&mut host, many, features()).is_err());
    host.app
        .plugins
        .instances
        .get_mut(&0)
        .unwrap()
        .application
        .retained_payload = api::MAX_RETAINED_BYTES;
    assert!(register(&mut host, vec![cmd.clone()], features()).is_err());
    assert!(host.app.plugins.commands.is_empty());
    assert!(!host.app.plugins.instances[&0].registered);
    host.app
        .plugins
        .instances
        .get_mut(&0)
        .unwrap()
        .application
        .retained_payload = 0;
    register(&mut host, vec![cmd], features()).unwrap();
    assert!(host.app.plugins.instances[&0].application.retained_payload > 0);
    assert!(host.app.plugins.instances[&0].registered);
}
