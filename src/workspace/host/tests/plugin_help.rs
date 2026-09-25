// SPDX-License-Identifier: MPL-2.0
use super::*;
use crate::command::HelpInvocation;
use serde_json::{Value, json};

fn topics() -> Value {
    json!([
        {"id":"rows","title":"Rows","paragraphs":[
            "Rows shows one page of a table. {key:nope} stays literal.",
            "Paging keeps the filter."
        ]},
        {"id":"record","title":"Record","paragraphs":["A record shows every field of one row."]}
    ])
}

fn registration(features: Value, help_topics: Option<Value>) -> api::ClientMessage {
    let mut message = json!({
        "type":"register","version":api::VERSION,
        "runyte":format!("={}", crate::plugin::compatibility::HOST_VERSION),
        "name":"Database viewer",
        "commands":[
            {"name":"open","description":"Open tables","context":"workspace"},
            {"name":"next-page","description":"Show the next page","context":"view",
             "presentation":{"label":"Next page","group":"Paging"}},
            {"name":"inspect","description":"Inspect {key:row}","context":"view","primary":true,
             "presentation":{"label":"Inspect {key:row} with Enter","listed":false}}
        ],
        "required_capabilities":["views"],"optional_capabilities":[],
        "required_features":[],"optional_features":features
    });
    if let Some(help_topics) = help_topics {
        message["help_topics"] = help_topics;
    }
    wire(&message).unwrap()
}

/// Decodes a frame the way a plugin's stdout is decoded, envelope checks
/// included.
fn wire(message: &Value) -> anyhow::Result<api::ClientMessage> {
    match api::decode(&serde_json::to_vec(message).unwrap())? {
        crate::plugin::ClientMessage::Application(message) => Ok(message),
        _ => anyhow::bail!("not an application message"),
    }
}

fn helped(host: &mut WorkspaceHost) -> mpsc::Receiver<HostMessage> {
    let mut cfg = config("tasks");
    cfg.capabilities = vec!["views".into()];
    // A configured binding puts the plugin's own description in the key table.
    cfg.bindings.insert("inspect".into(), "F5".into());
    let mut receiver = instance(host, 0, cfg);
    host.application_message(
        0,
        registration(
            json!([api::VIEW_HELP, api::VIEW_ACTION_PRESENTATION]),
            Some(topics()),
        ),
    )
    .unwrap();
    let api::HostMessage::Registered { features, .. } = next(&mut receiver) else {
        panic!()
    };
    assert!(features.contains(api::VIEW_HELP));
    host.app.note_plugin_frontend(true);
    receiver
}

fn page(help: Option<&str>) -> crate::plugin::view::Model {
    let mut model = json!({"title":"Rows","purpose":"list",
        "rows":[{"id":"one","text":"first","role":"ordinary"}]});
    if let Some(help) = help {
        model["help"] = json!(help);
    }
    serde_json::from_value(model).unwrap()
}

/// Opens contextual help, returns its text and highlight spans, and jumps
/// back to the view it described.
fn help_page(host: &mut WorkspaceHost) -> (String, Vec<crate::syntax::Span>) {
    host.app
        .execute(CommandInvocation::help(HelpInvocation::ActiveView))
        .unwrap();
    let buffer = host.app.active().buffer;
    let text = host.app.active_buffer().text().to_string();
    let spans = host.app.generated_highlights(buffer).to_vec();
    host.app
        .execute(
            CommandInvocation::editor(EditorCommand::JumpBackward, Default::default()).unwrap(),
        )
        .unwrap();
    (text, spans)
}

fn open_help(host: &mut WorkspaceHost) -> String {
    help_page(host).0
}

fn failure(receiver: &mut mpsc::Receiver<HostMessage>) -> api::ErrorCode {
    let api::HostMessage::Response {
        outcome: api::Response::Failure { error },
        ..
    } = next(receiver)
    else {
        panic!()
    };
    error.code
}

#[tokio::test]
async fn space_question_follows_the_models_registered_topic_and_live_actions() {
    let (_root, mut host) = host();
    let mut receiver = helped(&mut host);
    model_request(
        &mut host,
        0,
        1,
        api::Request::ViewCreate {
            model: page(Some("rows")),
        },
    )
    .await;
    let (view, revision) = view_result(&mut receiver);
    show_view(&mut host, &mut receiver, &view, 2);
    assert_eq!(
        host.app.key_binding_scope(),
        crate::keymap::BindingScope::Plugin(0)
    );

    let (rows, spans) = help_page(&mut host);
    assert!(rows.starts_with("Help · DATABASE VIEWER · ROWS · Read-only\n"));
    assert!(rows.contains("Rows shows one page of a table. {key:nope} stays literal."));
    assert!(!rows.contains("selection-first modal editor"));
    assert!(rows.contains("Application actions\n"));
    assert!(rows.contains("\n  Paging\n    Next page — Show the next page\n"));
    // Hidden from the menu, so hidden from the actions here; the key table
    // still documents the configured binding by its label. That label once
    // reached key-marker resolution unescaped and panicked the host.
    let actions =
        &rows[rows.find("Application actions").unwrap()..rows.find("Buffer keys").unwrap()];
    assert!(!actions.contains("Inspect"));
    assert!(rows.contains("\n  F5          Inspect {key:row} with Enter\n"));
    // The plugin's words stay plain; only the key cell is a key.
    let line = rows.find("  F5          Inspect").unwrap();
    let key = rows[..line + 2].chars().count();
    let label = rows[..rows.find(" with Enter\n").unwrap()].chars().count();
    let styled = |from: usize, to: usize| {
        spans
            .iter()
            .filter(|span| span.from < to && span.to > from)
            .map(|span| span.scope.name())
            .collect::<Vec<_>>()
    };
    assert_eq!(styled(key, key + 2), ["keyword"]);
    assert!(styled(key + 2, label + " with Enter".len()).is_empty());

    // A header patch replaces the topic, as it replaces the rest of the header.
    model_request(
        &mut host,
        0,
        3,
        serde_json::from_value(json!({"method":"view.patch","params":{
            "view":view,"expected_revision":revision,
            "header":{"title":"Record","purpose":"list","help":"record"},"operations":[]
        }}))
        .unwrap(),
    )
    .await;
    let (_, revision) = view_result(&mut receiver);
    let record = open_help(&mut host);
    assert!(record.starts_with("Help · DATABASE VIEWER · RECORD · Read-only\n"));
    assert!(record.contains("A record shows every field of one row."));

    // A model without a topic falls back to the ordinary overview.
    model_request(
        &mut host,
        0,
        4,
        api::Request::ViewPublish {
            expected_query_revision: None,
            view: view.clone(),
            expected_revision: revision.clone(),
            model: page(None),
        },
    )
    .await;
    let (_, revision) = view_result(&mut receiver);
    let fallback = open_help(&mut host);
    assert!(fallback.starts_with("Help · RUNYTE · TEXT · Read-only\n"));
    assert!(fallback.contains("selection-first modal editor"));
    assert!(fallback.contains("Next page — Show the next page"));

    // An unregistered topic refuses the whole publication and keeps the model.
    model_request(
        &mut host,
        0,
        5,
        api::Request::ViewPublish {
            expected_query_revision: None,
            view: view.clone(),
            expected_revision: revision,
            model: page(Some("value")),
        },
    )
    .await;
    assert_eq!(failure(&mut receiver), api::ErrorCode::InvalidArgument);
    assert!(open_help(&mut host).starts_with("Help · RUNYTE · TEXT"));

    // A stopped plugin's view stays readable, and help no longer quotes it.
    host.stop_plugin(0, "stopped by user");
    let stopped = open_help(&mut host);
    assert!(stopped.starts_with("Help · RUNYTE · TEXT"));
    assert!(!stopped.contains("Application actions"));
}

#[test]
fn help_topics_are_negotiated_validated_atomic_and_charged() {
    let (_root, mut host) = host();
    let mut cfg = config("tasks");
    cfg.capabilities = vec!["views".into()];
    let _receiver = instance(&mut host, 0, cfg.clone());
    // Authored topics, even none, need the feature.
    for help_topics in [topics(), json!([])] {
        assert!(
            host.application_message(0, registration(json!([]), Some(help_topics)))
                .is_err()
        );
    }
    let mut duplicate = topics();
    duplicate[1]["id"] = json!("rows");
    let mut oversized = topics();
    oversized[0]["paragraphs"] = json!(vec!["p"; 17]);
    for help_topics in [duplicate, oversized, json!(null)] {
        let message = json!({
            "type":"register","version":api::VERSION,
            "runyte":format!("={}", crate::plugin::compatibility::HOST_VERSION),
            "name":"Database viewer","commands":[{"name":"open","description":"Open","context":"workspace"}],
            "required_capabilities":["views"],"optional_capabilities":[],
            "required_features":[],"optional_features":[api::VIEW_HELP],
            "help_topics":help_topics
        });
        let refused = wire(&message).and_then(|message| host.application_message(0, message));
        assert!(refused.is_err());
    }
    let state = &host.app.plugins.instances[&0];
    assert!(!state.registered);
    assert!(state.application.help.is_none());
    assert!(host.app.plugins.commands.is_empty());

    // Help costs nothing unless topics were registered, and then exactly them.
    let (_root, mut plain) = super::host();
    let _receiver = instance(&mut plain, 0, cfg);
    plain
        .application_message(
            0,
            registration(json!([api::VIEW_ACTION_PRESENTATION, api::VIEW_HELP]), None),
        )
        .unwrap();
    let (_root, mut helped_host) = super::host();
    helped(&mut helped_host);
    let with = &helped_host.app.plugins.instances[&0].application;
    let without = &plain.app.plugins.instances[&0].application;
    assert!(without.help.is_none());
    let help = with.help.as_ref().unwrap();
    assert_eq!(help.application, "Database viewer");
    assert_eq!(help.topics.len(), 2);
    assert_eq!(
        with.retained_payload,
        without.retained_payload + help.payload_bytes()
    );
}

#[tokio::test]
async fn model_help_without_the_feature_is_unsupported() {
    let (_root, mut host) = host();
    let mut cfg = config("tasks");
    cfg.capabilities = vec!["views".into()];
    let mut receiver = instance(&mut host, 0, cfg);
    host.application_message(
        0,
        registration(json!([api::VIEW_ACTION_PRESENTATION]), None),
    )
    .unwrap();
    next(&mut receiver);
    model_request(
        &mut host,
        0,
        1,
        api::Request::ViewCreate {
            model: page(Some("rows")),
        },
    )
    .await;
    assert_eq!(failure(&mut receiver), api::ErrorCode::Unsupported);
    // `null` is not an absent topic.
    assert!(
        serde_json::from_value::<crate::plugin::view::Model>(
            json!({"title":"Rows","purpose":"list","rows":[],"help":null})
        )
        .is_err()
    );
}
