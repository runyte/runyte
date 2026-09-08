// SPDX-License-Identifier: MPL-2.0

use super::*;
use crate::plugin::application as api;

#[path = "plugin_activity.rs"]
mod activity;

#[path = "plugin_document_jobs.rs"]
mod document_jobs;
#[path = "plugin_documents.rs"]
mod documents;
#[path = "plugin_filesystem.rs"]
mod filesystem;
#[path = "plugin_filesystem_apply.rs"]
mod filesystem_apply;
#[path = "plugin_filesystem_recursive.rs"]
mod filesystem_recursive;
#[path = "plugin_filesystem_review.rs"]
mod filesystem_review;
#[path = "plugin_filesystem_stat.rs"]
mod filesystem_stat;
#[path = "plugin_handoffs.rs"]
mod handoffs;
#[path = "plugin_interaction.rs"]
mod interaction;
#[path = "plugin_model_review.rs"]
mod model_review;
#[path = "plugin_models.rs"]
mod models;
#[path = "plugin_notifications.rs"]
mod notifications;
#[path = "plugin_observations.rs"]
mod observations;
#[path = "plugin_processes.rs"]
mod processes;
#[path = "plugin_provider_inspect.rs"]
mod provider_inspect;
#[path = "plugin_provider_overwrite.rs"]
mod provider_overwrite;
#[path = "plugin_provider_rebind.rs"]
mod provider_rebind;
#[path = "plugin_provider_release.rs"]
mod provider_release;
#[path = "plugin_provider_review.rs"]
mod provider_review;
#[path = "plugin_provider_writes.rs"]
mod provider_writes;
#[path = "plugin_providers.rs"]
mod providers;
#[path = "plugin_settings.rs"]
mod settings;
#[path = "plugin_staging.rs"]
mod staging;
#[path = "plugin_state.rs"]
mod state;
#[path = "plugin_validation.rs"]
mod validation;
#[path = "plugin_validation_review.rs"]
mod validation_review;
#[path = "plugin_view_queries.rs"]
mod view_queries;
#[path = "plugin_view_query_review.rs"]
mod view_query_review;

fn setup(
    host: &mut WorkspaceHost,
    id: usize,
    capabilities: &[&str],
) -> mpsc::Receiver<HostMessage> {
    let mut cfg = config(&format!("app-{id}"));
    cfg.api = api::Api::Epoch2;
    cfg.capabilities = capabilities.iter().map(|s| (*s).into()).collect();
    let receiver = instance(host, id, cfg);
    host.application_message(
        id,
        api::ClientMessage::Register {
            settings_schema: None,
            version: api::VERSION.into(),
            name: "Tasks".into(),
            commands: vec![api::Registration {
                arguments: vec![],
                primary: false,
                name: "open".into(),
                description: "Open tasks".into(),
                context: api::CommandContext::Workspace,
            }],
            required_capabilities: capabilities.iter().map(|s| (*s).into()).collect(),
            optional_capabilities: Default::default(),
        },
    )
    .unwrap();
    receiver
}
fn next(receiver: &mut mpsc::Receiver<HostMessage>) -> api::HostMessage {
    loop {
        match receiver.try_recv().unwrap() {
            HostMessage::Application(api::HostMessage::Event {
                event: "resource.released",
                ..
            }) => {}
            HostMessage::Application(message) => return message,
            HostMessage::Deadline { .. } => {}
            other => panic!("unexpected {other:?}"),
        }
    }
}
fn request(host: &mut WorkspaceHost, plugin: usize, n: u64, request: api::Request) {
    host.handle_plugin_event(Event {
        plugin,
        result: Ok(ClientMessage::Application(api::ClientMessage::Request {
            id: format!("p:{n}"),
            request,
        })),
    });
}
fn job(receiver: &mut mpsc::Receiver<HostMessage>) -> api::Job {
    match next(receiver) {
        api::HostMessage::Response {
            outcome:
                api::Response::Success {
                    result: api::ResultValue::Job(job),
                },
            ..
        } => job,
        other => panic!("unexpected {other:?}"),
    }
}

fn type_command(host: &mut WorkspaceHost, text: &str) {
    use crate::input::KeyStroke;
    for character in format!(":{text}").chars() {
        host.app.handle_key(KeyStroke::char(character)).unwrap();
    }
    host.app
        .handle_key(KeyStroke::parse("Enter").unwrap())
        .unwrap();
}

#[test]
fn application_command_palette_submits_both_epochs_on_enter() {
    let (_root, mut host) = host();
    let mut legacy = instance(&mut host, 1, config("case"));
    register(&mut host, 1).unwrap();
    legacy.try_recv().unwrap();
    type_command(&mut host, "plugin.case.upper");
    assert_eq!(host.app.mode, crate::app::Mode::Normal);
    assert!(
        matches!(legacy.try_recv().unwrap(), HostMessage::Invoke { command, .. } if command == "upper")
    );

    let mut application = setup(&mut host, 0, &["views"]);
    next(&mut application);
    type_command(&mut host, "plugin.app-0.open");
    assert_eq!(host.app.mode, crate::app::Mode::Normal);
    assert!(
        matches!(next(&mut application), api::HostMessage::Request { params, .. } if params.command == "open")
    );
}

#[test]
fn application_palette_validates_arguments_before_closing_and_submits_quoted_values() {
    use crate::input::KeyStroke;
    let (_root, mut host) = host();
    let mut cfg = config("args");
    cfg.api = api::Api::Epoch2;
    let mut receiver = instance(&mut host, 0, cfg);
    host.application_message(
        0,
        api::ClientMessage::Register {
            settings_schema: None,
            version: api::VERSION.into(),
            name: "Arguments".into(),
            commands: vec![api::Registration {
                name: "echo".into(),
                description: "Echo arguments".into(),
                context: api::CommandContext::Workspace,
                primary: false,
                arguments: serde_json::from_str(
                    r#"[{"name":"label","type":"string"},{"name":"count","type":"integer"}]"#,
                )
                .unwrap(),
            }],
            required_capabilities: Default::default(),
            optional_capabilities: Default::default(),
        },
    )
    .unwrap();
    next(&mut receiver);
    type_command(&mut host, "plugin.args.echo 'é hello' bad");
    assert_eq!(host.app.mode, crate::app::Mode::Command);
    assert!(receiver.try_recv().is_err());
    host.app
        .handle_key(KeyStroke::parse("Esc").unwrap())
        .unwrap();
    type_command(&mut host, "plugin.args.echo 'é hello' -7");
    assert_eq!(host.app.mode, crate::app::Mode::Normal);
    let api::HostMessage::Request { params, .. } = next(&mut receiver) else {
        panic!()
    };
    assert_eq!(
        serde_json::to_value(params.arguments).unwrap(),
        serde_json::json!({"label":"é hello","count":-7})
    );
}

#[test]
fn application_invocation_captures_usable_buffer_and_pane_handles() {
    let (_root, mut host) = host();
    seed(&mut host, "é before");
    let mut receiver = setup(&mut host, 0, &["text", "selections"]);
    next(&mut receiver);
    invoke(&mut host, "plugin.app-0.open");
    let api::HostMessage::Request { params, .. } = next(&mut receiver) else {
        panic!()
    };
    let captured_buffer = params.buffer.unwrap();
    let captured_revision = params.buffer_revision.unwrap();
    host.app
        .execute(CommandInvocation::editor(EditorCommand::NewBuffer, Default::default()).unwrap())
        .unwrap();
    request(
        &mut host,
        0,
        1,
        api::Request::BufferRead {
            buffer: captured_buffer,
            expected_revision: captured_revision,
            from: 0,
            to: 8,
        },
    );
    assert!(matches!(next(&mut receiver), api::HostMessage::Response {
        outcome: api::Response::Success { result: api::ResultValue::Text { text, .. } }, ..
    } if text == "é before"));
    request(
        &mut host,
        0,
        2,
        api::Request::SelectionGet { pane: params.pane },
    );
    assert!(matches!(next(&mut receiver), api::HostMessage::Response {
        outcome: api::Response::Success { result: api::ResultValue::Selection { revision, .. } }, ..
    } if revision != params.selection_revision));
}

#[test]
fn jobs_survive_commands_and_protect_host_until_terminal_response() {
    let (_root, mut host) = host();
    let mut receiver = setup(&mut host, 0, &["workspace", "jobs"]);
    assert!(matches!(
        next(&mut receiver),
        api::HostMessage::Registered { .. }
    ));
    assert!(host.may_retire_idle());
    invoke(&mut host, "plugin.app-0.open");
    let api::HostMessage::Request { id: first, .. } = next(&mut receiver) else {
        panic!()
    };
    invoke(&mut host, "plugin.app-0.open");
    let api::HostMessage::Request { id: second, .. } = next(&mut receiver) else {
        panic!()
    };
    request(
        &mut host,
        0,
        1,
        api::Request::JobCreate {
            title: "Download".into(),
            deadline_seconds: 120,
        },
    );
    let created = job(&mut receiver);
    assert!(matches!(
        next(&mut receiver),
        api::HostMessage::Event {
            event: "job.changed",
            ..
        }
    ));
    assert_eq!(host.protected_state().plugin_jobs, 1);
    assert!(!host.may_retire_idle());
    assert!(host.protected_state().refusal().contains("plugin jobs"));
    for request in [second, first] {
        host.application_message(
            0,
            api::ClientMessage::Response {
                id: request,
                outcome: api::CommandResponse::Success {
                    result: api::CommandResult {
                        job: Some(created.job.clone()),
                    },
                },
            },
        )
        .unwrap();
    }
    request(
        &mut host,
        0,
        2,
        api::Request::JobFinish {
            job: created.job.clone(),
            state: api::TerminalState::Succeeded,
        },
    );
    assert_eq!(job(&mut receiver).state, api::JobState::Succeeded);
    assert!(matches!(
        next(&mut receiver),
        api::HostMessage::Event {
            event: "job.changed",
            ..
        }
    ));
    assert!(host.may_retire_idle());
    request(
        &mut host,
        0,
        3,
        api::Request::JobFinish {
            job: created.job,
            state: api::TerminalState::Succeeded,
        },
    );
    assert!(matches!(
        next(&mut receiver),
        api::HostMessage::Response {
            outcome: api::Response::Failure {
                error: api::Error {
                    code: api::ErrorCode::Conflict,
                    ..
                }
            },
            ..
        }
    ));
    assert!(receiver.try_recv().is_err());
}

#[test]
fn cancellation_is_idempotent_rejects_late_success_and_preserves_other_owner() {
    let (_root, mut host) = host();
    let mut a = setup(&mut host, 0, &["jobs"]);
    let mut b = setup(&mut host, 1, &["jobs"]);
    next(&mut a);
    next(&mut b);
    request(
        &mut host,
        0,
        1,
        api::Request::JobCreate {
            title: "Upload".into(),
            deadline_seconds: 30,
        },
    );
    let created = job(&mut a);
    next(&mut a);
    request(
        &mut host,
        1,
        1,
        api::Request::JobGet {
            job: created.job.clone(),
        },
    );
    assert!(matches!(
        next(&mut b),
        api::HostMessage::Response {
            outcome: api::Response::Failure {
                error: api::Error {
                    code: api::ErrorCode::NotFound,
                    ..
                }
            },
            ..
        }
    ));
    request(
        &mut host,
        0,
        2,
        api::Request::JobCancel {
            job: created.job.clone(),
        },
    );
    assert_eq!(job(&mut a).state, api::JobState::Cancelling);
    assert!(matches!(
        next(&mut a),
        api::HostMessage::Event {
            event: "job.cancel_requested",
            ..
        }
    ));
    request(
        &mut host,
        0,
        3,
        api::Request::JobCancel {
            job: created.job.clone(),
        },
    );
    job(&mut a);
    assert!(a.try_recv().is_err());
    request(
        &mut host,
        0,
        4,
        api::Request::JobFinish {
            job: created.job.clone(),
            state: api::TerminalState::Succeeded,
        },
    );
    assert!(matches!(
        next(&mut a),
        api::HostMessage::Response {
            outcome: api::Response::Failure {
                error: api::Error {
                    code: api::ErrorCode::Cancelled,
                    ..
                }
            },
            ..
        }
    ));
    host.app
        .plugins
        .instances
        .get_mut(&0)
        .unwrap()
        .application
        .deadlines
        .remove(&created.job);
    host.handle_plugin_event(Event {
        plugin: 0,
        result: Ok(ClientMessage::Deadline { token: created.job }),
    });
    assert!(!host.app.plugins.instances.contains_key(&0));
    assert!(host.app.plugins.instances.contains_key(&1));
    assert!(!host.app.plugins.commands.values().any(|c| c.plugin == 0));
}

#[test]
fn required_capabilities_registration_rollback_and_request_reuse_are_bounded() {
    let (_root, mut host) = host();
    let mut a = setup(&mut host, 0, &[]);
    next(&mut a);
    request(&mut host, 0, 1, api::Request::WorkspaceInfo(api::Empty {}));
    assert!(matches!(
        next(&mut a),
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
    request(&mut host, 0, 1, api::Request::WorkspaceInfo(api::Empty {}));
    assert!(!host.app.plugins.instances.contains_key(&0));
    let mut cfg = config("denied");
    cfg.api = api::Api::Epoch2;
    let _receiver = instance(&mut host, 2, cfg);
    assert!(
        host.application_message(
            2,
            api::ClientMessage::Register {
                settings_schema: None,
                version: api::VERSION.into(),
                name: "Denied".into(),
                commands: vec![],
                required_capabilities: ["jobs".into()].into(),
                optional_capabilities: Default::default(),
            }
        )
        .is_err()
    );
    assert!(!host.app.plugins.commands.values().any(|c| c.plugin == 2));
}

#[test]
fn generations_and_job_limits_prevent_reuse_and_unbounded_work() {
    let (_root, mut host) = host();
    let mut a = setup(&mut host, 0, &["jobs"]);
    next(&mut a);
    let mut old = String::new();
    for n in 1..=4 {
        request(
            &mut host,
            0,
            n,
            api::Request::JobCreate {
                title: "Copy".into(),
                deadline_seconds: 3600,
            },
        );
        old = job(&mut a).job;
        next(&mut a);
    }
    request(
        &mut host,
        0,
        5,
        api::Request::JobCreate {
            title: "Overflow".into(),
            deadline_seconds: 3600,
        },
    );
    assert!(matches!(
        next(&mut a),
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
    host.stop_plugin(0, "stopped by user");
    let mut a = setup(&mut host, 0, &["jobs"]);
    next(&mut a);
    request(&mut host, 0, 1, api::Request::JobGet { job: old.clone() });
    assert!(matches!(
        next(&mut a),
        api::HostMessage::Response {
            outcome: api::Response::Failure { .. },
            ..
        }
    ));
    request(
        &mut host,
        0,
        2,
        api::Request::JobCreate {
            title: "New".into(),
            deadline_seconds: 30,
        },
    );
    assert_ne!(job(&mut a).job, old);
}

#[tokio::test]
async fn real_application_job_outlives_control_timeout_and_attachment() {
    let (root, mut host) = host();
    let program = root.join("jobs");
    std::os::unix::fs::symlink(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/fixtures/stand-in"),
        &program,
    )
    .unwrap();
    std::fs::write(root.join("jobs.behavior"), r#"
printf 'started\n' >> "$0.starts"
read -r hello
printf '%s\n' '{"type":"register","version":"runyte-experimental-2","name":"Jobs","commands":[{"name":"open","description":"Start job","context":"workspace"}],"required_capabilities":["jobs"],"optional_capabilities":[]}'
read -r registered
read -r invocation
printf '%s\n' '{"type":"request","id":"p:1","method":"job.create","params":{"title":"Long task","deadline_seconds":60}}'
read -r created
read -r event
printf '%s\n' '{"type":"response","id":"h:1","result":{"job":null}}'
sleep 11
printf '%s\n' '{"type":"request","id":"p:2","method":"job.create","params":{"title":"Still alive","deadline_seconds":60}}'
while read -r message; do :; done
"#).unwrap();
    let mut cfg = config("jobs");
    cfg.api = api::Api::Epoch2;
    cfg.executable = program;
    cfg.capabilities = vec!["jobs".into()];
    host.app.config.plugins.push(cfg);
    let mut events = host.start_plugins().unwrap();
    for phase in 0..4 {
        let event = tokio::time::timeout(std::time::Duration::from_secs(15), events.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(event.result.is_ok(), "{:?}", event.result);
        host.handle_plugin_event(event);
        if phase == 0 {
            invoke(&mut host, "plugin.jobs.open");
        }
        if phase == 2 {
            host.app.note_frontend_attached();
            host.app.note_frontend_attached();
            assert!(host.start_plugins().is_none());
            assert_eq!(host.protected_state().plugin_jobs, 1);
        }
    }
    assert_eq!(host.protected_state().plugin_jobs, 2);
    assert_eq!(
        std::fs::read_to_string(root.join("jobs.starts")).unwrap(),
        "started\n"
    );
    host.stop_plugin(0, "stopped by user");
    assert!(host.plugin_workers.is_empty());
}

fn model_service(host: &mut WorkspaceHost) -> mpsc::Receiver<Event> {
    let (sender, receiver) = mpsc::channel(plugin::EVENT_CAPACITY);
    host.plugin_events_sender = Some(sender);
    receiver
}

async fn model_complete(host: &mut WorkspaceHost, events: &mut mpsc::Receiver<Event>) -> bool {
    let event = tokio::time::timeout(std::time::Duration::from_secs(5), events.recv())
        .await
        .unwrap()
        .unwrap();
    host.handle_plugin_event(event)
}

async fn model_request(host: &mut WorkspaceHost, owner: usize, n: u64, operation: api::Request) {
    assert!(
        host.app
            .plugins
            .instances
            .values()
            .all(|instance| instance.application.model_requests.is_empty())
    );
    let mut events = model_service(host);
    request(host, owner, n, operation);
    while host
        .app
        .plugins
        .instances
        .get(&owner)
        .is_some_and(|instance| {
            instance
                .application
                .model_requests
                .contains_key(&format!("p:{n}"))
        })
    {
        model_complete(host, &mut events).await;
    }
}

fn model(rows: &[(&str, &str)]) -> crate::plugin::view::Model {
    crate::plugin::view::Model {
        title: "Tasks".into(),
        purpose: crate::plugin::view::Purpose::List,
        rows: rows
            .iter()
            .map(|(id, text)| crate::plugin::view::Row {
                id: (*id).into(),
                text: (*text).into(),
                role: crate::plugin::view::Role::Ordinary,
                ..Default::default()
            })
            .collect(),
        ..Default::default()
    }
}
fn view_setup(host: &mut WorkspaceHost) -> mpsc::Receiver<HostMessage> {
    let mut cfg = config("tasks");
    cfg.api = api::Api::Epoch2;
    cfg.capabilities = vec!["views".into()];
    let mut receiver = instance(host, 0, cfg);
    host.application_message(
        0,
        api::ClientMessage::Register {
            settings_schema: None,
            version: api::VERSION.into(),
            name: "Tasks".into(),
            commands: vec![
                api::Registration {
                    arguments: vec![],
                    name: "open".into(),
                    description: "Open tasks".into(),
                    context: api::CommandContext::Workspace,
                    primary: false,
                },
                api::Registration {
                    arguments: vec![],
                    name: "toggle".into(),
                    description: "Toggle selected tasks".into(),
                    context: api::CommandContext::View,
                    primary: true,
                },
            ],
            required_capabilities: ["views".into()].into(),
            optional_capabilities: Default::default(),
        },
    )
    .unwrap();
    next(&mut receiver);
    host.app.note_plugin_frontend(true);
    receiver
}
fn view_result(receiver: &mut mpsc::Receiver<HostMessage>) -> (String, String) {
    let api::HostMessage::Response {
        outcome:
            api::Response::Success {
                result: api::ResultValue::View { view, revision, .. },
            },
        ..
    } = next(receiver)
    else {
        panic!()
    };
    (view, revision)
}
fn show_view(
    host: &mut WorkspaceHost,
    receiver: &mut mpsc::Receiver<HostMessage>,
    view: &str,
    n: u64,
) {
    invoke(host, "plugin.tasks.open");
    let api::HostMessage::Request { id, .. } = next(receiver) else {
        panic!()
    };
    request(
        host,
        0,
        n,
        api::Request::PaneShow {
            invocation: id,
            view: view.into(),
        },
    );
    assert!(matches!(
        next(receiver),
        api::HostMessage::Response {
            outcome: api::Response::Success { .. },
            ..
        }
    ));
}

#[tokio::test]
async fn native_view_preserves_split_selections_by_row_and_rejects_unseen_actions() {
    let (_root, mut host) = host();
    let mut receiver = view_setup(&mut host);
    model_request(
        &mut host,
        0,
        1,
        api::Request::ViewCreate {
            model: model(&[("one", "α first"), ("two", "second"), ("three", "third")]),
        },
    )
    .await;
    let (view, revision) = view_result(&mut receiver);
    assert_eq!(host.app.active().buffer, 0, "creation cannot take focus");
    show_view(&mut host, &mut receiver, &view, 2);
    let buffer = host.app.active().buffer;
    assert!(host.app.buffers[buffer].is_read_only());
    host.app
        .execute(CommandInvocation::split_vertical(None))
        .unwrap();
    assert_eq!(
        host.app
            .panes
            .values()
            .filter(|p| p.buffer == buffer)
            .count(),
        2
    );
    let second = host.app.buffers[buffer].line_to_offset(1);
    for pane in host.app.panes.values_mut().filter(|p| p.buffer == buffer) {
        pane.selection = Selection::single(Range::new(second + 3, second + 1));
    }
    host.app.prepare_view(Default::default());
    model_request(
        &mut host,
        0,
        3,
        api::Request::ViewPublish {
            expected_query_revision: None,
            view: view.clone(),
            expected_revision: revision,
            model: model(&[("three", "third"), ("one", "α first"), ("two", "second")]),
        },
    )
    .await;
    view_result(&mut receiver);
    let second = host.app.buffers[buffer].line_to_offset(2);
    for pane in host.app.panes.values().filter(|p| p.buffer == buffer) {
        assert_eq!(pane.selection.primary(), Range::new(second + 3, second + 1));
    }
    assert!(!host.app.buffers[buffer].dirty);
    assert!(matches!(
        invoke(&mut host, "plugin.tasks.toggle"),
        CommandOutcome::UserError(_)
    ));
    host.app.prepare_view(Default::default());
    assert!(matches!(
        invoke(&mut host, "plugin.tasks.toggle"),
        CommandOutcome::AsynchronousRequest(_)
    ));
    let api::HostMessage::Request { params, .. } = next(&mut receiver) else {
        panic!()
    };
    assert_eq!(params.rows, vec!["two"]);
    assert_eq!(params.model_revision.as_deref(), Some("m:2"));
    assert_eq!(
        host.app.key_binding_scope(),
        crate::keymap::BindingScope::Plugin(0)
    );
    assert!(matches!(
        host.app.keymap().lookup_in(
            crate::command::Mode::Normal,
            host.app.key_binding_scope(),
            &crate::keymap::KeySequence::parse("Enter").unwrap()
        ),
        crate::keymap::Lookup::Exact(_)
    ));
}

#[tokio::test]
async fn presentation_grant_expires_on_input_and_detach_but_hidden_models_keep_updating() {
    let (_root, mut host) = host();
    let mut receiver = view_setup(&mut host);
    model_request(
        &mut host,
        0,
        1,
        api::Request::ViewCreate {
            model: model(&[("one", "original")]),
        },
    )
    .await;
    let (view, revision) = view_result(&mut receiver);
    invoke(&mut host, "plugin.tasks.open");
    let api::HostMessage::Request { id: invocation, .. } = next(&mut receiver) else {
        panic!()
    };
    host.app
        .execute(CommandInvocation::editor(EditorCommand::MoveRight, Default::default()).unwrap())
        .unwrap();
    request(
        &mut host,
        0,
        2,
        api::Request::PaneShow {
            invocation: invocation.clone(),
            view: view.clone(),
        },
    );
    assert!(matches!(
        next(&mut receiver),
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
    host.app.note_plugin_frontend(false);
    request(
        &mut host,
        0,
        3,
        api::Request::PaneShow {
            invocation,
            view: view.clone(),
        },
    );
    assert!(matches!(
        next(&mut receiver),
        api::HostMessage::Response {
            outcome: api::Response::Failure {
                error: api::Error {
                    code: api::ErrorCode::NoFrontend,
                    ..
                }
            },
            ..
        }
    ));
    let mut events = model_service(&mut host);
    let changed = host.handle_plugin_event(Event {
        plugin: 0,
        result: Ok(ClientMessage::Application(api::ClientMessage::Request {
            id: "p:4".into(),
            request: api::Request::ViewPublish {
                expected_query_revision: None,
                view: view.clone(),
                expected_revision: revision,
                model: model(&[("one", "updated")]),
            },
        })),
    });
    assert!(!changed, "hidden publication must not request a frame");
    assert!(
        !model_complete(&mut host, &mut events).await,
        "hidden completion must not request a frame"
    );
    view_result(&mut receiver);
    host.app.note_plugin_frontend(true);
    show_view(&mut host, &mut receiver, &view, 5);
    assert_eq!(host.app.active_buffer().to_string(), "updated\n");
    host.stop_plugin(0, "stopped by user");
    assert_eq!(host.app.active_buffer().to_string(), "updated\n");
    assert!(
        host.app
            .active_buffer()
            .display_name()
            .contains("unavailable")
    );
}

#[tokio::test]
async fn invalid_model_is_atomic_and_close_releases_owned_handle_after_response() {
    let (_root, mut host) = host();
    let mut receiver = view_setup(&mut host);
    model_request(
        &mut host,
        0,
        1,
        api::Request::ViewCreate {
            model: model(&[("one", "original")]),
        },
    )
    .await;
    let (view, revision) = view_result(&mut receiver);
    model_request(
        &mut host,
        0,
        2,
        api::Request::ViewPublish {
            expected_query_revision: None,
            view: view.clone(),
            expected_revision: revision,
            model: model(&[("one", "changed"), ("one", "duplicate")]),
        },
    )
    .await;
    assert!(matches!(
        next(&mut receiver),
        api::HostMessage::Response {
            outcome: api::Response::Failure { .. },
            ..
        }
    ));
    let buffer = host.app.plugins.instances[&0].application.views[&view].buffer;
    assert_eq!(host.app.buffers[buffer].to_string(), "original\n");
    request(
        &mut host,
        0,
        3,
        api::Request::ViewClose { view: view.clone() },
    );
    assert!(matches!(
        next(&mut receiver),
        api::HostMessage::Response {
            outcome: api::Response::Success { .. },
            ..
        }
    ));
    assert!(matches!(
        next(&mut receiver),
        api::HostMessage::Event {
            event: "view.closed",
            ..
        }
    ));
    assert!(
        !host.app.plugins.instances[&0]
            .application
            .views
            .contains_key(&view)
    );
}

#[test]
fn explicit_unicode_edits_are_atomic_hidden_and_one_undo_while_snapshots_stay_immutable() {
    let (_root, mut host) = host();
    seed(&mut host, "éß 😀xy");
    let mut receiver = setup(&mut host, 0, &["workspace", "text", "selections"]);
    next(&mut receiver);
    request(
        &mut host,
        0,
        1,
        api::Request::BufferList {
            offset: 0,
            limit: 32,
        },
    );
    let api::HostMessage::Response {
        outcome:
            api::Response::Success {
                result: api::ResultValue::Buffers { buffers, .. },
            },
        ..
    } = next(&mut receiver)
    else {
        panic!()
    };
    let buffer = buffers[0].buffer.clone();
    let revision = buffers[0].revision.clone();
    request(
        &mut host,
        0,
        2,
        api::Request::SnapshotOpen {
            buffer: buffer.clone(),
            expected_revision: revision.clone(),
        },
    );
    let api::HostMessage::Response {
        outcome:
            api::Response::Success {
                result: api::ResultValue::Snapshot { snapshot, .. },
            },
        ..
    } = next(&mut receiver)
    else {
        panic!()
    };
    host.app
        .execute(CommandInvocation::editor(EditorCommand::NewBuffer, Default::default()).unwrap())
        .unwrap();
    let hidden_focus = host.app.active().buffer;
    assert_ne!(hidden_focus, 0);
    request(
        &mut host,
        0,
        3,
        api::Request::BufferEdit {
            buffer: buffer.clone(),
            expected_revision: revision.clone(),
            changes: vec![
                crate::plugin::editor::Change {
                    from: 0,
                    to: 2,
                    text: "ÉSS".into(),
                },
                crate::plugin::editor::Change {
                    from: 3,
                    to: 4,
                    text: "🐈".into(),
                },
            ],
        },
    );
    assert!(matches!(
        next(&mut receiver),
        api::HostMessage::Response {
            outcome: api::Response::Success {
                result: api::ResultValue::Edited { changes: 2, .. }
            },
            ..
        }
    ));
    assert_eq!(host.app.active().buffer, hidden_focus);
    assert_eq!(host.app.buffers[0].to_string(), "ÉSS 🐈xy");
    request(
        &mut host,
        0,
        4,
        api::Request::BufferRead {
            buffer: buffer.clone(),
            expected_revision: revision.clone(),
            from: 0,
            to: 2,
        },
    );
    assert!(matches!(
        next(&mut receiver),
        api::HostMessage::Response {
            outcome: api::Response::Failure {
                error: api::Error {
                    code: api::ErrorCode::Stale,
                    ..
                }
            },
            ..
        }
    ));
    request(
        &mut host,
        0,
        5,
        api::Request::SnapshotRead {
            snapshot: snapshot.clone(),
            from: 0,
            to: 4,
        },
    );
    let api::HostMessage::Response {
        outcome:
            api::Response::Success {
                result:
                    api::ResultValue::Text {
                        text,
                        revision: old_revision,
                        ..
                    },
            },
        ..
    } = next(&mut receiver)
    else {
        panic!()
    };
    assert_eq!(text, "éß 😀");
    assert_eq!(revision, old_revision);
    host.app
        .panes
        .get_mut(&host.app.active_pane)
        .unwrap()
        .buffer = 0;
    undo(&mut host);
    assert_eq!(host.app.buffers[0].to_string(), "éß 😀xy");
    let revision = format!("r:{}", host.app.buffers[0].revision());
    request(
        &mut host,
        0,
        6,
        api::Request::BufferEdit {
            buffer,
            expected_revision: revision,
            changes: vec![
                crate::plugin::editor::Change {
                    from: 0,
                    to: 3,
                    text: "bad".into(),
                },
                crate::plugin::editor::Change {
                    from: 2,
                    to: 4,
                    text: "overlap".into(),
                },
            ],
        },
    );
    assert!(matches!(
        next(&mut receiver),
        api::HostMessage::Response {
            outcome: api::Response::Failure {
                error: api::Error {
                    code: api::ErrorCode::InvalidArgument,
                    ..
                }
            },
            ..
        }
    ));
    assert_eq!(host.app.buffers[0].to_string(), "éß 😀xy");
    request(&mut host, 0, 7, api::Request::SnapshotClose { snapshot });
    assert!(matches!(
        next(&mut receiver),
        api::HostMessage::Response {
            outcome: api::Response::Success { .. },
            ..
        }
    ));
    assert_eq!(
        host.app.plugins.instances[&0].application.retained_payload,
        0
    );
}

#[test]
fn pane_selection_targets_revision_and_foreign_buffer_handles_are_rejected() {
    let (_root, mut host) = host();
    seed(&mut host, "éß 😀xy");
    let mut receiver = setup(&mut host, 0, &["workspace", "text", "selections"]);
    next(&mut receiver);
    request(&mut host, 0, 1, api::Request::PaneList(api::Empty {}));
    let api::HostMessage::Response {
        outcome:
            api::Response::Success {
                result: api::ResultValue::Panes { panes },
            },
        ..
    } = next(&mut receiver)
    else {
        panic!()
    };
    let pane = panes[0].pane.clone();
    let buffer = panes[0].buffer.clone().unwrap();
    let revision = panes[0].selection_revision.clone();
    request(
        &mut host,
        0,
        2,
        api::Request::SelectionSet {
            pane: pane.clone(),
            buffer: buffer.clone(),
            expected_revision: revision.clone(),
            ranges: vec![crate::plugin::editor::Range { anchor: 5, head: 3 }],
            primary: 0,
        },
    );
    let api::HostMessage::Response {
        outcome:
            api::Response::Success {
                result: api::ResultValue::Selection { ranges, .. },
            },
        ..
    } = next(&mut receiver)
    else {
        panic!()
    };
    assert_eq!((ranges[0].anchor, ranges[0].head), (5, 3));
    request(
        &mut host,
        0,
        3,
        api::Request::SelectionSet {
            pane,
            buffer: buffer.clone(),
            expected_revision: revision,
            ranges: vec![crate::plugin::editor::Range { anchor: 0, head: 0 }],
            primary: 0,
        },
    );
    assert!(matches!(
        next(&mut receiver),
        api::HostMessage::Response {
            outcome: api::Response::Failure {
                error: api::Error {
                    code: api::ErrorCode::Stale,
                    ..
                }
            },
            ..
        }
    ));
    let current_revision = format!("r:{}", host.app.buffers[0].revision());
    let mut other = setup(&mut host, 1, &["text"]);
    next(&mut other);
    request(
        &mut host,
        1,
        1,
        api::Request::BufferRead {
            buffer,
            expected_revision: current_revision,
            from: 0,
            to: 1,
        },
    );
    assert!(matches!(
        next(&mut other),
        api::HostMessage::Response {
            outcome: api::Response::Failure {
                error: api::Error {
                    code: api::ErrorCode::NotFound,
                    ..
                }
            },
            ..
        }
    ));
}

#[tokio::test]
async fn native_view_survives_private_frame_round_trip_with_theme_roles_in_narrow_panes() {
    let (_root, mut host) = host();
    let mut receiver = view_setup(&mut host);
    let mut model = model(&[("warn", "Careful"), ("error", "Failed")]);
    model.rows[0].role = crate::plugin::view::Role::Warning;
    model.rows[1].role = crate::plugin::view::Role::Error;
    model_request(&mut host, 0, 1, api::Request::ViewCreate { model }).await;
    let (view, _) = view_result(&mut receiver);
    show_view(&mut host, &mut receiver, &view, 2);
    let geometry = crate::ui::frame_geometry(ratatui::layout::Rect::new(0, 0, 40, 12));
    let core = host.prepare_frame(geometry);
    let wire: crate::protocol::HostFrame = core.into();
    let bytes = serde_json::to_vec(&wire).unwrap();
    let wire: crate::protocol::HostFrame = serde_json::from_slice(&bytes).unwrap();
    let core: crate::workspace::HostFrame = wire.try_into().unwrap();
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(40, 12)).unwrap();
    terminal
        .draw(|frame| crate::ui::render_host_frame_exact_colors_for_test(frame, &core))
        .unwrap();
    let cells = terminal.backend().buffer();
    let rendered = cells
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    assert!(rendered.contains("Careful"));
    assert!(rendered.contains("Failed"));
    let theme = host
        .app
        .config
        .resolve_theme(
            host.app
                .config
                .theme
                .as_deref()
                .unwrap_or(crate::config::DEFAULT_THEME),
        )
        .unwrap();
    for (word, expected) in [("Careful", theme.warning), ("Failed", theme.error)] {
        let crate::config::Color::Rgb(red, green, blue) = expected else {
            panic!("default theme uses RGB");
        };
        let start = cells
            .content
            .windows(word.len())
            .position(|window| window.iter().map(|cell| cell.symbol()).collect::<String>() == word)
            .unwrap();
        // The first character can carry the normal-mode caret; the rest uses the semantic role.
        assert_eq!(
            cells.content[start + 1].fg,
            ratatui::style::Color::Rgb(red, green, blue)
        );
    }
}

#[tokio::test]
async fn view_actions_use_half_open_row_selections_in_both_directions() {
    let (_root, mut host) = host();
    let mut output = view_setup(&mut host);
    model_request(
        &mut host,
        0,
        1,
        api::Request::ViewCreate {
            model: model(&[
                ("one", "α first"),
                ("two", "second"),
                ("three", "third"),
                ("four", "fourth"),
            ]),
        },
    )
    .await;
    let (view, _) = view_result(&mut output);
    show_view(&mut host, &mut output, &view, 2);
    let buffer = host.app.active().buffer;
    let second = host.app.buffers[buffer].line_to_offset(1);
    let third = host.app.buffers[buffer].line_to_offset(2);
    let fourth = host.app.buffers[buffer].line_to_offset(3);
    for (ranges, expected) in [
        (vec![Range::new(0, second)], vec!["one"]),
        (vec![Range::new(second, 0)], vec!["one"]),
        (vec![Range::new(second, second)], vec!["two"]),
        (
            vec![Range::new(0, second), Range::new(fourth, third)],
            vec!["one", "three"],
        ),
        (vec![Range::new(0, third)], vec!["one", "two"]),
    ] {
        host.app
            .panes
            .get_mut(&host.app.active_pane)
            .unwrap()
            .selection = Selection::new(ranges, 0);
        host.app.prepare_view(Default::default());
        assert!(matches!(
            invoke(&mut host, "plugin.tasks.toggle"),
            CommandOutcome::AsynchronousRequest(_)
        ));
        let api::HostMessage::Request { id, params, .. } = next(&mut output) else {
            panic!()
        };
        assert_eq!(params.rows, expected);
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
    }
}

#[test]
fn snapshot_close_cannot_clear_command_or_job_deadlines() {
    let (_root, mut host) = host();
    let mut output = setup(&mut host, 0, &["text", "jobs"]);
    next(&mut output);
    invoke(&mut host, "plugin.app-0.open");
    let api::HostMessage::Request { id: command, .. } = next(&mut output) else {
        panic!()
    };
    assert!(
        matches!(output.try_recv().unwrap(), HostMessage::Deadline { token, after_ms: Some(10000) } if token == command)
    );
    request(
        &mut host,
        0,
        1,
        api::Request::JobCreate {
            title: "Slow".into(),
            deadline_seconds: 1,
        },
    );
    let created = job(&mut output);
    next(&mut output);
    for (index, token) in [&command, &created.job].into_iter().enumerate() {
        let due = host.app.plugins.instances[&0]
            .application
            .deadlines
            .get(token)
            .copied();
        request(
            &mut host,
            0,
            index as u64 + 2,
            api::Request::SnapshotClose {
                snapshot: token.clone(),
            },
        );
        assert!(matches!(
            output.try_recv().unwrap(),
            HostMessage::Application(api::HostMessage::Response {
                outcome: api::Response::Success { .. },
                ..
            })
        ));
        assert_eq!(
            host.app.plugins.instances[&0]
                .application
                .deadlines
                .get(token),
            due.as_ref()
        );
    }
    host.app
        .plugins
        .instances
        .get_mut(&0)
        .unwrap()
        .application
        .deadlines
        .insert(created.job.clone(), std::time::Instant::now());
    host.handle_plugin_event(Event {
        plugin: 0,
        result: Ok(ClientMessage::Deadline { token: created.job }),
    });
    assert!(matches!(
        next(&mut output),
        api::HostMessage::Event {
            event: "job.cancel_requested",
            ..
        }
    ));
    host.app
        .plugins
        .instances
        .get_mut(&0)
        .unwrap()
        .application
        .deadlines
        .insert(command.clone(), std::time::Instant::now());
    host.handle_plugin_event(Event {
        plugin: 0,
        result: Ok(ClientMessage::Deadline { token: command }),
    });
    assert!(!host.app.plugins.instances.contains_key(&0));
}

#[test]
fn terminal_job_history_retains_completion_order_across_handle_widths() {
    let (_root, mut host) = host();
    let mut output = setup(&mut host, 0, &["jobs"]);
    next(&mut output);
    let mut completed = std::collections::VecDeque::new();
    for index in 0..110 {
        request(
            &mut host,
            0,
            index * 2 + 1,
            api::Request::JobCreate {
                title: "Work".into(),
                deadline_seconds: 60,
            },
        );
        let created = job(&mut output);
        next(&mut output);
        request(
            &mut host,
            0,
            index * 2 + 2,
            api::Request::JobFinish {
                job: created.job.clone(),
                state: api::TerminalState::Succeeded,
            },
        );
        job(&mut output);
        next(&mut output);
        completed.push_back(created.job);
        if completed.len() > 64 {
            completed.pop_front();
        }
        let state = &host.app.plugins.instances[&0].application;
        assert_eq!(state.finished_jobs, completed);
        assert_eq!(
            state
                .jobs
                .keys()
                .cloned()
                .collect::<std::collections::BTreeSet<_>>(),
            completed.iter().cloned().collect()
        );
    }
}
