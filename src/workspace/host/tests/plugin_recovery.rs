// SPDX-License-Identifier: MPL-2.0
use super::provider_writes::document;
use super::providers::{metadata, reply, resource_request};
use super::*;
use crate::plugin::provider as wire;

fn begin(
    host: &mut WorkspaceHost,
    output: &mut mpsc::Receiver<HostMessage>,
    index: usize,
    remote: &str,
) -> String {
    host.app.note_plugin_frontend(true);
    host.app.show_provider_document(index);
    type_command(host, "reload");
    host.sync_provider_recoveries();
    let job = host
        .provider_reads
        .keys()
        .next()
        .unwrap_or_else(|| panic!("Reload admission failed: {}", host.app.status))
        .clone();
    let (id, operation) = resource_request(output);
    assert!(matches!(operation, wire::Request::Stat { .. }));
    assert!(matches!(
        next(output),
        api::HostMessage::Event {
            event: "job.changed",
            ..
        }
    ));
    let mut stat = metadata(remote.len());
    stat.version = "remote-v2".into();
    reply(host, id, wire::Response::Stat(stat));
    let (id, operation) = resource_request(output);
    assert!(matches!(operation, wire::Request::Read { .. }));
    reply(
        host,
        id,
        wire::Response::Read(wire::Chunk {
            version: "remote-v2".into(),
            offset: 0,
            text: remote.into(),
            eof: true,
        }),
    );
    job
}
fn key(host: &mut WorkspaceHost, name: &str) {
    host.app
        .handle_input(crate::input::InputEvent::Key(
            crate::input::KeyStroke::parse(name).unwrap(),
        ))
        .unwrap();
}

#[tokio::test]
async fn provider_reload_clean_adopts_fresh_bytes_without_a_choice_or_open_event() {
    let (_root, mut host) = host();
    let (_requester, mut provider, handle, index) = document(&mut host, "base");
    let (sender, mut observer) = mpsc::channel(32);
    let instance = host.app.plugins.instances.get_mut(&0).unwrap();
    instance.sender = plugin::Sender::new(sender);
    instance.application.capabilities.insert("workspace".into());
    request(
        &mut host,
        0,
        900,
        api::Request::EventSubscribe {
            sources: vec![crate::plugin::observation::Source::Buffer { buffer: handle }],
        },
    );
    let initial = serde_json::to_value(next(&mut observer)).unwrap();
    assert!(initial.get("error").is_none(), "{initial}");
    let initial_revision = initial["result"]["sources"][0]["state"]["revision"].clone();
    let mut events = model_service(&mut host);
    begin(&mut host, &mut provider, index, "remote 猫\r\n");
    assert_eq!(host.app.buffers[index].to_string(), "base");
    assert!(host.app.document_mutation_pending(index));
    model_complete(&mut host, &mut events).await;
    let update = serde_json::to_value(next(&mut observer)).unwrap();
    assert_eq!(update["event"], "event.changed");
    let state = &update["data"]["sources"][0]["state"];
    assert_ne!(state["revision"], initial_revision);
    assert_eq!(state["revision"], state["saved_revision"]);
    assert_eq!(state["dirty"], false);
    assert_eq!(host.app.buffers[index].to_string(), "remote 猫\r\n");
    assert!(!host.app.buffers[index].dirty);
    assert_eq!(
        host.app.buffers[index].provider().unwrap().version,
        "remote-v2"
    );
    assert_eq!(
        host.app.buffers[index].provider().unwrap().baseline_epoch,
        1
    );
    assert!(host.app.plugins.provider_reload.is_none());
    assert!(host.provider_reads.is_empty() && host.plugin_recoveries.is_empty());
    assert!(!host.app.document_mutation_pending(index));
    assert_eq!(
        host.app.plugins.instances[&1].application.retained_payload,
        0
    );
    assert!(host.app.status.contains("Reloaded remote text"));
    while let Ok(message) = provider.try_recv() {
        assert!(!matches!(
            message,
            HostMessage::Application(api::HostMessage::Event {
                event: "resource.opened",
                ..
            })
        ));
    }
}

#[tokio::test]
async fn provider_reload_dirty_choices_preserve_text_until_physical_acceptance() {
    for choice in ["replace", "keep", "cancel"] {
        let (_root, mut host) = host();
        let (_requester, mut provider, _, index) = document(&mut host, "base");
        host.app.buffers[index].apply(&Transaction::insert(0, "local "));
        let revision = host.app.buffers[index].revision();
        let mut events = model_service(&mut host);
        begin(&mut host, &mut provider, index, "remote");
        model_complete(&mut host, &mut events).await;
        assert!(host.app.plugins.provider_reload.is_some());
        assert_eq!(host.app.buffers[index].revision(), revision);
        assert_eq!(
            host.app.buffers[index].provider().unwrap().version,
            "version-1"
        );
        if choice != "cancel" {
            key(&mut host, "Up");
        }
        if choice == "replace" {
            key(&mut host, "Up");
        }
        key(&mut host, "Enter");
        host.sync_provider_recoveries();
        assert!(host.provider_reads.is_empty() && host.plugin_recoveries.is_empty());
        assert!(!host.app.document_mutation_pending(index));
        assert_eq!(
            host.app.plugins.instances[&1].application.retained_payload,
            0
        );
        assert_eq!(
            host.app.buffers[index].to_string(),
            if choice == "replace" {
                "remote"
            } else {
                "local base"
            }
        );
        assert_eq!(
            host.app.buffers[index].provider().unwrap().version,
            if choice == "cancel" {
                "version-1"
            } else {
                "remote-v2"
            }
        );
        assert_eq!(host.app.buffers[index].dirty, choice != "replace");
        assert!(host.app.status.contains(match choice {
            "replace" => "Reloaded remote text",
            "keep" => "Kept local edits and accepted the remote baseline",
            _ => "Remote reload cancelled",
        }));
    }
}

#[tokio::test]
async fn provider_reload_stale_text_discards_prepared_result_and_keeps_baseline() {
    let (_root, mut host) = host();
    let (_requester, mut provider, _, index) = document(&mut host, "base");
    let mut events = model_service(&mut host);
    begin(&mut host, &mut provider, index, "remote");
    host.app.buffers[index].apply(&Transaction::insert(0, "later "));
    model_complete(&mut host, &mut events).await;
    assert_eq!(host.app.buffers[index].to_string(), "later base");
    assert_eq!(
        host.app.buffers[index].provider().unwrap().version,
        "version-1"
    );
    assert!(host.app.buffers[index].dirty);
    assert!(host.plugin_recoveries.is_empty());
    assert_eq!(
        host.app.plugins.instances[&1].application.retained_payload,
        0
    );
}

#[tokio::test]
async fn provider_reload_cancelled_worker_protects_standalone_until_actual_completion() {
    use std::sync::{Arc, Condvar, Mutex};
    struct Gate(Mutex<bool>, Condvar);
    struct Release(Arc<Gate>);
    impl Drop for Release {
        fn drop(&mut self) {
            *self.0.0.lock().unwrap() = true;
            self.0.1.notify_all();
        }
    }
    let (_root, mut host) = host();
    let (_requester, mut provider, _, index) = document(&mut host, "base");
    let gate = Arc::new(Gate(Mutex::new(false), Condvar::new()));
    let release = Release(gate.clone());
    let (started, ready) = tokio::sync::oneshot::channel();
    let started = Mutex::new(Some(started));
    host.plugin_recovery_hook = Some(Arc::new(move || {
        let _ = started.lock().unwrap().take().unwrap().send(());
        let mut open = gate.0.lock().unwrap();
        while !*open {
            open = gate.1.wait(open).unwrap();
        }
    }));
    let mut events = model_service(&mut host);
    let job = begin(&mut host, &mut provider, index, "remote");
    tokio::time::timeout(std::time::Duration::from_secs(3), ready)
        .await
        .unwrap()
        .unwrap();
    host.finish_provider_read(
        &job,
        Err(api::Error::new(
            api::ErrorCode::Cancelled,
            "Remote reload cancelled",
        )),
    );
    assert!(!host.app.document_mutation_pending(index));
    assert_eq!(host.app.plugins.provider_reload_cleanup, 1);
    assert!(host.app.plugin_active_job_count() > 0);
    assert_eq!(
        host.app.plugins.instances[&1].application.retained_payload,
        32 * 1024 * 1024 + 2 * "base".len()
    );
    drop(release);
    model_complete(&mut host, &mut events).await;
    assert_eq!(host.app.plugins.provider_reload_cleanup, 0);
    assert_eq!(
        host.app.plugins.instances[&1].application.retained_payload,
        0
    );
    assert_eq!(host.app.buffers[index].to_string(), "base");
    assert_eq!(
        host.app.buffers[index].provider().unwrap().version,
        "version-1"
    );
}

#[tokio::test]
async fn provider_reload_ready_choice_expires_or_retires_with_provider_before_acceptance() {
    for stop in [false, true] {
        let (_root, mut host) = host();
        let (_requester, mut provider, _, index) = document(&mut host, "base");
        host.app.buffers[index].apply(&Transaction::insert(0, "local "));
        let mut events = model_service(&mut host);
        let job = begin(&mut host, &mut provider, index, "remote");
        model_complete(&mut host, &mut events).await;
        assert!(host.app.plugins.provider_reload.is_some());
        if stop {
            host.stop_plugin(1, "stopped by user");
        } else {
            assert!(host.provider_deadline(1, &job));
        }
        assert!(host.app.plugins.provider_reload.is_none());
        assert!(host.plugin_recoveries.is_empty() && host.provider_reads.is_empty());
        host.sync_provider_recoveries();
        assert_eq!(host.app.buffers[index].to_string(), "local base");
        assert_eq!(
            host.app.buffers[index].provider().unwrap().version,
            "version-1"
        );
        assert_eq!(
            host.app.buffers[index].provider().unwrap().baseline_epoch,
            0
        );
        assert!(host.app.buffers[index].dirty);
        assert!(!host.app.document_mutation_pending(index));
        assert_eq!(host.app.plugins.orphaned_payload, 0);
        assert_eq!(host.app.plugins.provider_reload_cleanup, 0);
    }
}

#[tokio::test]
async fn provider_reload_small_document_coexists_with_retained_remote_browser_view() {
    let (_root, mut host) = host();
    let (_requester, mut provider, _, index) = document(&mut host, "base");
    host.app
        .plugins
        .instances
        .get_mut(&1)
        .unwrap()
        .application
        .capabilities
        .insert("views".into());
    model_request(
        &mut host,
        1,
        901,
        api::Request::ViewCreate {
            model: model(&[("notes", "Remote notes"), ("other", "Other document")]),
        },
    )
    .await;
    let (view, revision) = view_result(&mut provider);
    let view_charge = host.app.plugins.instances[&1].application.retained_payload;
    assert!(view_charge > 0);
    let mut events = model_service(&mut host);
    begin(&mut host, &mut provider, index, "fresh");
    assert_eq!(
        host.app.plugins.instances[&1].application.retained_payload,
        view_charge + 32 * 1024 * 1024 + 2 * "base".len()
    );
    model_complete(&mut host, &mut events).await;
    assert_eq!(host.app.buffers[index].to_string(), "fresh");
    assert!(!host.app.buffers[index].dirty);
    let state = &host.app.plugins.instances[&1].application;
    assert_eq!(state.retained_payload, view_charge);
    assert_eq!(format!("m:{}", state.views[&view].revision), revision);
    assert_eq!(state.views[&view].model.rows.len(), 2);
}
