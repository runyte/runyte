// SPDX-License-Identifier: MPL-2.0

use super::*;
use crate::{
    app::{App, CommandOutcome, HostPorts},
    clipboard::SystemClipboard,
    command::{CommandExecutionContext, CommandInvocation, EditorCommand},
    plugin::{PluginConfig, Registration},
    selection::{Range, Selection},
    test_support::TestRuntimeRoot,
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
        api: Default::default(),
        capabilities: vec![],
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
            pending: None,
            issued: BTreeSet::new(),
            subscriptions: Default::default(),
            sequence: 0,
        },
    );
    receiver
}
fn register(host: &mut WorkspaceHost, id: usize) -> Result<()> {
    host.plugin_message(
        id,
        ClientMessage::Register {
            version: plugin::VERSION.into(),
            commands: vec![Registration {
                name: "upper".into(),
                description: "Uppercase selections".into(),
            }],
        },
    )
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
fn reply(host: &mut WorkspaceHost, replacements: &[&str]) {
    let invocation = host.app.plugins.instances[&0]
        .pending
        .as_ref()
        .unwrap()
        .token
        .clone();
    host.handle_plugin_event(Event {
        plugin: 0,
        result: Ok(ClientMessage::Replace {
            invocation,
            replacements: replacements.iter().map(|s| (*s).to_owned()).collect(),
        }),
    });
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
fn unicode_multiselection_capture_focus_mapping_and_single_undo_without_frame() {
    let (root, mut host) = host();
    seed(&mut host, "éß 😀xy");
    let mut receiver = instance(&mut host, 0, config("case"));
    register(&mut host, 0).unwrap();
    receiver.try_recv().unwrap();
    host.app.panes.get_mut(&0).unwrap().selection =
        Selection::new(vec![Range::new(1, 0), Range::new(3, 5)], 1);
    assert!(matches!(
        invoke(&mut host, "plugin.case.upper"),
        CommandOutcome::AsynchronousRequest(_)
    ));
    let HostMessage::Invoke {
        text,
        selections,
        primary,
        buffer,
        ..
    } = receiver.try_recv().unwrap()
    else {
        panic!()
    };
    assert_eq!(text, "éß 😀xy");
    assert_eq!(primary, 1);
    assert_eq!(
        (
            selections[0].anchor,
            selections[0].head,
            selections[0].from,
            selections[0].to
        ),
        (1, 0, 0, 2)
    );
    assert_eq!((selections[1].from, selections[1].to), (3, 6));
    assert_eq!(buffer, "0");
    assert!(host.current_frame_id().is_none());
    host.app
        .execute(crate::command::parse_colon_command("vsplit").unwrap())
        .unwrap();
    assert_ne!(host.app.active_pane, 0);
    let path = root.join("other.txt");
    std::fs::write(&path, "other").unwrap();
    let other = host.open_buffer(path, true).unwrap();
    reply(&mut host, &["ÉSS", "😀XY"]);
    assert_eq!(host.app.buffers[0].to_string(), "ÉSS 😀XY");
    assert_eq!(BufferId::from_index(host.app.active().buffer), other);
    assert_eq!(host.app.active_buffer().to_string(), "other");
    assert!(matches!(
        receiver.try_recv().unwrap(),
        HostMessage::Complete {
            status: "applied",
            revision: Some(_),
            ..
        }
    ));
    assert_eq!(host.app.panes[&0].buffer, 0);
    host.app.active_pane = 0;
    undo(&mut host);
    assert_eq!(host.app.buffers[0].to_string(), "éß 😀xy");
    undo(&mut host);
    assert_eq!(host.app.buffers[0].to_string(), "");
}

#[test]
fn stale_after_edit_and_undo_closed_and_bad_replacements_are_atomic() {
    for scenario in ["edit", "undo", "closed", "count", "readonly"] {
        let (_root, mut host) = host();
        seed(&mut host, "abc");
        let mut receiver = instance(&mut host, 0, config("case"));
        register(&mut host, 0).unwrap();
        receiver.try_recv().unwrap();
        invoke(&mut host, "plugin.case.upper");
        receiver.try_recv().unwrap();
        match scenario {
            "edit" | "undo" => {
                let revision = host.app.buffers[0].revision();
                host.apply_expected_transaction(
                    BufferId::from_index(0),
                    BufferRevision::from_raw(revision),
                    Transaction::insert(3, "!"),
                )
                .unwrap();
                if scenario == "undo" {
                    undo(&mut host);
                }
            }
            "closed" => host.close_buffer(BufferId::from_index(0), true).unwrap(),
            "readonly" => {
                host.app.buffers[0].kind = crate::buffer::BufferKind::Help;
            }
            _ => {}
        }
        let before = host.app.buffers[0].to_string();
        reply(&mut host, if scenario == "count" { &[] } else { &["X"] });
        assert_eq!(host.app.buffers[0].to_string(), before, "{scenario}");
        let HostMessage::Complete { status, .. } = receiver.try_recv().unwrap() else {
            panic!()
        };
        assert_eq!(
            status,
            match scenario {
                "edit" | "undo" => "stale",
                "closed" => "closed",
                "readonly" => "read_only",
                _ => "invalid_replacements",
            },
            "{scenario}"
        );
    }
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
        HostMessage::Invoke { .. }
    ));
    assert!(matches!(
        invoke(&mut host, "plugin.case.upper"),
        CommandOutcome::UserError(_)
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

#[test]
fn subscriptions_order_revision_coalescing_unsubscribe_closure_and_slow_consumer() {
    let (_root, mut host) = host();
    seed(&mut host, "abc");
    let mut receiver = instance(&mut host, 0, config("case"));
    register(&mut host, 0).unwrap();
    receiver.try_recv().unwrap();
    invoke(&mut host, "plugin.case.upper");
    receiver.try_recv().unwrap();
    host.plugin_message(
        0,
        ClientMessage::Subscribe {
            request: "s".into(),
            buffer: "0".into(),
        },
    )
    .unwrap();
    assert!(
        matches!(receiver.try_recv().unwrap(), HostMessage::Subscribed { revision, .. } if revision == host.app.buffers[0].revision().to_string())
    );
    reply(&mut host, &["A"]);
    assert!(matches!(
        receiver.try_recv().unwrap(),
        HostMessage::Complete {
            status: "applied",
            ..
        }
    ));
    assert!(
        matches!(receiver.try_recv().unwrap(), HostMessage::BufferState { sequence, revision, closed: false, .. } if sequence == "1" && revision == host.app.buffers[0].revision().to_string())
    );
    host.sync_plugin_observers();
    assert!(receiver.try_recv().is_err());
    undo(&mut host);
    host.sync_plugin_observers();
    assert!(
        matches!(receiver.try_recv().unwrap(), HostMessage::BufferState { sequence, revision, .. } if sequence == "2" && revision == host.app.buffers[0].revision().to_string())
    );
    host.plugin_message(
        0,
        ClientMessage::Unsubscribe {
            request: "u".into(),
            buffer: "0".into(),
        },
    )
    .unwrap();
    assert!(matches!(
        receiver.try_recv().unwrap(),
        HostMessage::Unsubscribed { .. }
    ));
    undo(&mut host);
    host.sync_plugin_observers();
    assert!(receiver.try_recv().is_err());
    host.plugin_message(
        0,
        ClientMessage::Subscribe {
            request: "s2".into(),
            buffer: "0".into(),
        },
    )
    .unwrap();
    receiver.try_recv().unwrap();
    host.close_buffer(BufferId::from_index(0), true).unwrap();
    host.sync_plugin_observers();
    assert!(matches!(
        receiver.try_recv().unwrap(),
        HostMessage::BufferState { closed: true, .. }
    ));
    // Each response is bounded; the ninth unanswered control request fails the
    // plugin instead of retaining an unbounded stream or blocking the editor.
    for i in 0..9 {
        host.handle_plugin_event(Event {
            plugin: 0,
            result: Ok(ClientMessage::Unsubscribe {
                request: i.to_string(),
                buffer: "0".into(),
            }),
        });
    }
    assert!(!host.app.plugins.instances.contains_key(&0));
    assert!(host.app.plugins.commands.is_empty());
}

#[tokio::test]
async fn disabled_spawn_failure_and_host_attachment_do_not_duplicate_instances() {
    let (_root, mut host) = host();
    let mut disabled = config("off");
    disabled.enabled = false;
    host.app.config.plugins.push(disabled);
    assert!(host.start_plugins().is_none());
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

#[cfg(unix)]
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
    std::fs::write(root.join("case.behavior"), r#"
printf 'started\n' >> "$0.starts"
read -r hello
printf '%s\n' '{"type":"register","version":"runyte-experimental-1","commands":[{"name":"upper","description":"Uppercase selections"}]}'
read -r registered
while read -r request; do
    case "$request" in
        *'"type":"invoke"'*) printf '%s\n' '{"type":"replace","invocation":"1","replacements":["ÉSS"]}' ;;
    esac
done
"#).unwrap();
    let mut cfg = config("case");
    cfg.executable = program;
    host.app.config.plugins.push(cfg);
    let mut events = host.start_plugins().unwrap();
    let event = tokio::time::timeout(std::time::Duration::from_secs(3), events.recv())
        .await
        .unwrap()
        .unwrap();
    host.handle_plugin_event(event);
    host.app.panes.get_mut(&0).unwrap().selection = Selection::single(Range::new(0, 2));
    invoke(&mut host, "plugin.case.upper");
    host.app.note_frontend_attached();
    assert!(host.start_plugins().is_none());
    let event = tokio::time::timeout(std::time::Duration::from_secs(3), events.recv())
        .await
        .unwrap()
        .unwrap();
    host.handle_plugin_event(event);
    assert_eq!(host.app.buffers[0].to_string(), "ÉSS");
    assert_eq!(
        std::fs::read_to_string(root.join("case.starts")).unwrap(),
        "started\n"
    );
    undo(&mut host);
    assert_eq!(host.app.buffers[0].to_string(), "éß");
}

#[test]
fn plugin_edit_splits_an_existing_insert_undo_group() {
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
    receiver.try_recv().unwrap();
    invoke(&mut host, "plugin.case.upper");
    receiver.try_recv().unwrap();
    reply(&mut host, &["A"]);
    assert_eq!(host.app.buffers[0].to_string(), "Abc!");
    undo(&mut host);
    assert_eq!(host.app.buffers[0].to_string(), "abc!");
    undo(&mut host);
    assert_eq!(host.app.buffers[0].to_string(), "abc");
}

#[test]
fn invalid_registration_tokens_and_subscription_errors_are_structured() {
    for registration in [
        ClientMessage::Register {
            version: "future".into(),
            commands: vec![],
        },
        ClientMessage::Register {
            version: plugin::VERSION.into(),
            commands: vec![],
        },
        ClientMessage::Register {
            version: plugin::VERSION.into(),
            commands: vec![Registration {
                name: "stop".into(),
                description: "reserved".into(),
            }],
        },
        ClientMessage::Register {
            version: plugin::VERSION.into(),
            commands: vec![Registration {
                name: "upper".into(),
                description: "bad\nlabel".into(),
            }],
        },
    ] {
        let (_root, mut host) = host();
        let _receiver = instance(&mut host, 0, config("case"));
        host.handle_plugin_event(Event {
            plugin: 0,
            result: Ok(registration),
        });
        assert!(host.app.plugins.commands.is_empty());
        assert!(host.app.plugins.instances.is_empty());
    }
    let (_root, mut host) = host();
    let mut receiver = instance(&mut host, 0, config("case"));
    register(&mut host, 0).unwrap();
    receiver.try_recv().unwrap();
    host.plugin_message(
        0,
        ClientMessage::Subscribe {
            request: "q".into(),
            buffer: "999".into(),
        },
    )
    .unwrap();
    assert!(matches!(
        receiver.try_recv().unwrap(),
        HostMessage::Error {
            code: "unknown_buffer",
            ..
        }
    ));
    host.handle_plugin_event(Event {
        plugin: 0,
        result: Ok(ClientMessage::Replace {
            invocation: "unknown".into(),
            replacements: vec![],
        }),
    });
    assert!(host.app.plugins.instances.is_empty());
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
fn observer_overflow_or_queued_stop_can_retire_an_incoming_result() {
    for stop in [false, true] {
        let (_root, mut host) = host();
        seed(&mut host, "abc");
        let mut receiver = instance(&mut host, 0, config("case"));
        register(&mut host, 0).unwrap();
        receiver.try_recv().unwrap();
        invoke(&mut host, "plugin.case.upper");
        receiver.try_recv().unwrap();
        let token = host.app.plugins.instances[&0]
            .pending
            .as_ref()
            .unwrap()
            .token
            .clone();
        if stop {
            invoke(&mut host, "plugin.case.stop");
        } else {
            host.plugin_message(
                0,
                ClientMessage::Subscribe {
                    request: "s".into(),
                    buffer: "0".into(),
                },
            )
            .unwrap();
            receiver.try_recv().unwrap();
            for _ in 0..8 {
                host.plugin_send(
                    0,
                    HostMessage::Error {
                        request: "q".into(),
                        code: "unknown_buffer",
                    },
                )
                .unwrap();
            }
            let revision = host.app.buffers[0].revision();
            host.apply_expected_transaction(
                BufferId::from_index(0),
                BufferRevision::from_raw(revision),
                Transaction::insert(3, "!"),
            )
            .unwrap();
        }
        let before = host.app.buffers[0].to_string();
        host.handle_plugin_event(Event {
            plugin: 0,
            result: Ok(ClientMessage::Replace {
                invocation: token,
                replacements: vec!["X".into()],
            }),
        });
        assert_eq!(host.app.buffers[0].to_string(), before);
        assert!(host.app.plugins.instances.is_empty());
        assert!(host.app.plugins.commands.is_empty());
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
fn asynchronous_plugin_completion_updates_only_its_own_action_echo() {
    for intervening_key in [false, true] {
        let (_root, mut host) = host();
        seed(&mut host, "abc");
        let mut cfg = config("case");
        cfg.bindings.insert("upper".into(), "F12".into());
        let mut receiver = instance(&mut host, 0, cfg);
        register(&mut host, 0).unwrap();
        receiver.try_recv().unwrap();
        host.app
            .handle_input(crate::input::InputEvent::Key(
                crate::input::KeyStroke::parse("F12").unwrap(),
            ))
            .unwrap();
        receiver.try_recv().unwrap();
        assert!(host.app.displayed_status_message().contains("accepted"));
        if intervening_key {
            host.app
                .handle_input(crate::input::InputEvent::Key(
                    crate::input::KeyStroke::char('l'),
                ))
                .unwrap();
        }
        let before = host.app.displayed_status_message().to_owned();
        reply(&mut host, &["A"]);
        if intervening_key {
            assert_eq!(host.app.displayed_status_message(), before);
        } else {
            assert!(host.app.displayed_status_message().contains("applied"));
        }
        assert_eq!(host.app.buffers[0].to_string(), "Abc");
    }
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

#[path = "plugin_applications.rs"]
mod applications;

#[path = "plugin_manager.rs"]
mod manager;
#[path = "plugin_manager_review.rs"]
mod manager_review;
