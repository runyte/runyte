// SPDX-License-Identifier: MPL-2.0

use super::provider_writes::document;
use super::providers::{metadata, reply, resource_request};
use super::*;
use crate::plugin::provider as wire;
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

fn make_uncertain(host: &mut WorkspaceHost, index: usize, ledger: bool) {
    host.app.note_plugin_frontend(true);
    host.app.buffers[index].apply(&Transaction::insert(0, "uploaded "));
    let captured = host.app.buffers[index].prepare_provider_save().unwrap();
    assert!(host.app.buffers[index].mark_provider_uncertain(captured));
    if ledger {
        host.provider_uncertain.insert(
            index,
            ("app-0".into(), wire::READ_CHARGE, "j:previous-write".into()),
        );
        host.app.plugins.orphaned_payload += wire::READ_CHARGE;
    }
    host.app.show_provider_document(index);
}

fn begin(
    host: &mut WorkspaceHost,
    provider: &mut mpsc::Receiver<HostMessage>,
    index: usize,
) -> (String, wire::Request) {
    host.app.note_plugin_frontend(true);
    host.app.show_provider_document(index);
    type_command(host, "reload");
    host.sync_provider_recoveries();
    let request = resource_request(provider);
    assert!(matches!(
        next(provider),
        api::HostMessage::Event {
            event: "job.changed",
            ..
        }
    ));
    request
}

#[tokio::test]
async fn provider_recovery_requires_exact_settlement_before_read_or_baseline_changes() {
    for proof in ["missing", "stat", "wrong-token"] {
        let (_root, mut host) = host();
        let (_requester, mut provider, _, index) = document(&mut host, "base");
        make_uncertain(&mut host, index, proof != "missing");
        let revision = host.app.buffers[index].revision();
        if proof == "missing" {
            type_command(&mut host, "reload");
            host.sync_provider_recoveries();
            assert!(host.app.status.contains("settlement reference"));
        } else {
            let (id, request) = begin(&mut host, &mut provider, index);
            assert!(
                matches!(request, wire::Request::Reconcile { previous_write, .. } if previous_write == "j:previous-write")
            );
            let metadata = metadata(4);
            reply(
                &mut host,
                id,
                if proof == "stat" {
                    wire::Response::Stat(metadata)
                } else {
                    wire::Response::Reconciled {
                        metadata,
                        previous_write: "j:wrong-write".into(),
                    }
                },
            );
            assert!(host.app.status.contains("previous write is settled"));
        }
        assert!(host.plugin_recoveries.is_empty());
        assert!(host.provider_reads.is_empty());
        assert!(!host.app.document_mutation_pending(index));
        assert_eq!(host.app.buffers[index].revision(), revision);
        assert_eq!(host.app.buffers[index].to_string(), "uploaded base");
        let document = host.app.buffers[index].provider().unwrap();
        assert_eq!(document.version, "version-1");
        assert_eq!(document.baseline_epoch, 0);
        assert!(document.uncertain.is_some());
        assert!(host.app.buffers[index].dirty);
        assert_eq!(
            host.app.plugins.orphaned_payload,
            if proof == "missing" {
                0
            } else {
                wire::READ_CHARGE
            }
        );
        assert_eq!(
            host.app.plugins.instances[&1].application.retained_payload,
            0
        );
        while let Ok(message) = provider.try_recv() {
            assert!(
                !matches!(
                    message,
                    HostMessage::Application(api::HostMessage::ResourceRequest { .. })
                ),
                "unproved settlement must never dispatch a chunk read"
            );
        }
    }
}

struct Gate {
    open: Mutex<bool>,
    changed: Condvar,
}
impl Gate {
    fn release(&self) {
        *self.open.lock().unwrap() = true;
        self.changed.notify_all();
    }
}
struct Release(Arc<Gate>);
impl Drop for Release {
    fn drop(&mut self) {
        self.0.release();
    }
}

#[tokio::test]
async fn provider_recovery_stop_retains_worker_payload_for_unavailable_old_binding() {
    let (_root, mut host) = host();
    let (_requester, mut provider, _, index) = document(&mut host, "base");
    host.app.buffers[index].apply(&Transaction::insert(0, "local "));
    host.app.buffers[index].provider_mut().unwrap().available = false;
    let original = host.app.buffers[index].revision();
    let worker_charge =
        wire::READ_CHARGE + 16 * 1024 * 1024 + 2 * host.app.buffers[index].len_bytes();
    let gate = Arc::new(Gate {
        open: Mutex::new(false),
        changed: Condvar::new(),
    });
    let release = Release(gate.clone());
    let (started, ready) = tokio::sync::oneshot::channel();
    let started = Mutex::new(Some(started));
    host.plugin_recovery_hook = Some(Arc::new(move || {
        if let Some(started) = started.lock().unwrap().take() {
            let _ = started.send(());
        }
        let mut open = gate.open.lock().unwrap();
        while !*open {
            open = gate.changed.wait(open).unwrap();
        }
    }));
    let (events, mut completions) = mpsc::channel(plugin::EVENT_CAPACITY);
    host.plugin_events_sender = Some(events);
    let (id, request) = begin(&mut host, &mut provider, index);
    assert!(matches!(request, wire::Request::Stat { .. }));
    reply(&mut host, id, wire::Response::Stat(metadata(6)));
    let (id, request) = resource_request(&mut provider);
    assert!(matches!(request, wire::Request::Read { .. }));
    reply(
        &mut host,
        id,
        wire::Response::Read(wire::Chunk {
            version: "version-1".into(),
            offset: 0,
            text: "remote".into(),
            eof: true,
        }),
    );
    tokio::time::timeout(Duration::from_secs(3), ready)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        host.app.plugins.instances[&1].application.retained_payload,
        worker_charge
    );
    host.stop_plugin(1, "stopped by user");
    assert!(!host.app.plugins.instances.contains_key(&1));
    assert!(host.provider_reads.is_empty());
    assert_eq!(host.plugin_recoveries.len(), 1);
    assert_eq!(host.app.plugins.orphaned_payload, worker_charge);
    assert!(host.protected_state().plugin_jobs > 0);
    assert!(host.app.plugins.provider_reload.is_none());
    release.0.release();
    let event = tokio::time::timeout(Duration::from_secs(3), completions.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(event.result, Ok(ClientMessage::ProviderReload(_))));
    host.handle_plugin_event(event);
    assert!(host.plugin_recoveries.is_empty());
    assert_eq!(host.app.plugins.orphaned_payload, 0);
    assert_eq!(host.app.buffers[index].revision(), original);
    assert_eq!(host.app.buffers[index].to_string(), "local base");
    assert!(!host.app.buffers[index].provider().unwrap().available);
    assert_eq!(
        host.app.buffers[index].provider().unwrap().baseline_epoch,
        0
    );
    assert!(host.app.plugins.provider_reload.is_none());
}

#[tokio::test]
async fn provider_recovery_peak_refusal_preserves_unknown_snapshot_without_transport_calls() {
    for global in [false, true] {
        let (_root, mut host) = host();
        let (_requester, mut provider, _, index) = document(&mut host, "base");
        make_uncertain(&mut host, index, true);
        let required_charge = 16 * 1024 * 1024 + 2 * host.app.buffers[index].len_bytes();
        let mut others = Vec::new();
        if global {
            let final_charge =
                160 * 1024 * 1024 - wire::READ_CHARGE - required_charge - 96 * 1024 * 1024 + 1;
            for (owner, bytes) in [
                (2, 48 * 1024 * 1024),
                (3, 48 * 1024 * 1024),
                (4, final_charge),
            ] {
                let mut output = setup(&mut host, owner, &[]);
                next(&mut output);
                host.app
                    .plugins
                    .instances
                    .get_mut(&owner)
                    .unwrap()
                    .application
                    .retained_payload = bytes;
                others.push(output);
            }
        } else {
            host.app
                .plugins
                .instances
                .get_mut(&1)
                .unwrap()
                .application
                .retained_payload = 48 * 1024 * 1024 - required_charge + 1;
        }
        type_command(&mut host, "reload");
        host.sync_provider_recoveries();
        assert!(host.app.status_error);
        assert!(
            host.app
                .status
                .contains("Retained application payload quota exceeded"),
            "unexpected refusal: {}",
            host.app.status
        );
        assert!(host.plugin_recoveries.is_empty());
        assert!(host.provider_reads.is_empty());
        assert!(!host.app.document_mutation_pending(index));
        assert!(provider.try_recv().is_err());
        assert_eq!(host.app.plugins.orphaned_payload, wire::READ_CHARGE);
        assert!(
            host.app.buffers[index]
                .provider()
                .unwrap()
                .uncertain
                .is_some()
        );
        assert_eq!(host.app.buffers[index].to_string(), "uploaded base");
        assert_eq!(
            host.app.buffers[index].provider().unwrap().baseline_epoch,
            0
        );
        assert!(host.app.plugins.instances[&1].application.jobs.is_empty());
    }
}

#[tokio::test]
async fn provider_recovery_cancelled_worker_pins_uncertainty_charge_after_buffer_close() {
    let (_root, mut host) = host();
    let (_requester, mut provider, _, index) = document(&mut host, "base");
    make_uncertain(&mut host, index, true);
    let prepare_charge = 16 * 1024 * 1024 + 2 * host.app.buffers[index].len_bytes();
    let gate = Arc::new(Gate {
        open: Mutex::new(false),
        changed: Condvar::new(),
    });
    let release = Release(gate.clone());
    let (started, ready) = tokio::sync::oneshot::channel();
    let started = Mutex::new(Some(started));
    host.plugin_recovery_hook = Some(Arc::new(move || {
        if let Some(started) = started.lock().unwrap().take() {
            let _ = started.send(());
        }
        let mut open = gate.open.lock().unwrap();
        while !*open {
            open = gate.changed.wait(open).unwrap();
        }
    }));
    let (events, mut completions) = mpsc::channel(plugin::EVENT_CAPACITY);
    host.plugin_events_sender = Some(events);
    let (id, request) = begin(&mut host, &mut provider, index);
    assert!(matches!(request, wire::Request::Reconcile { .. }));
    let job = host.provider_reads.keys().next().unwrap().clone();
    reply(
        &mut host,
        id,
        wire::Response::Reconciled {
            metadata: metadata(6),
            previous_write: "j:previous-write".into(),
        },
    );
    let (id, request) = resource_request(&mut provider);
    assert!(matches!(request, wire::Request::Read { .. }));
    reply(
        &mut host,
        id,
        wire::Response::Read(wire::Chunk {
            version: "version-1".into(),
            offset: 0,
            text: "remote".into(),
            eof: true,
        }),
    );
    tokio::time::timeout(Duration::from_secs(3), ready)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(host.app.plugins.orphaned_payload, wire::READ_CHARGE);
    assert_eq!(
        host.app.plugins.instances[&1].application.retained_payload,
        prepare_charge
    );
    host.finish_provider_read(
        &job,
        Err(api::Error::new(
            api::ErrorCode::Cancelled,
            "Remote reload cancelled",
        )),
    );
    assert!(!host.app.document_mutation_pending(index));
    host.app.host_close_buffer(index, true).unwrap();
    assert!(host.app.host_buffer_is_closed(index));
    host.reconcile_provider_payload();
    assert!(host.provider_uncertain.contains_key(&index));
    assert_eq!(host.app.plugins.orphaned_payload, wire::READ_CHARGE);
    assert_eq!(host.app.plugins.provider_reload_cleanup, 1);
    assert_eq!(
        host.app.plugins.instances[&1].application.retained_payload,
        prepare_charge
    );
    drop(release);
    let event = tokio::time::timeout(Duration::from_secs(3), completions.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(event.result, Ok(ClientMessage::ProviderReload(_))));
    host.handle_plugin_event(event);
    assert!(host.plugin_recoveries.is_empty());
    assert!(!host.provider_uncertain.contains_key(&index));
    assert_eq!(host.app.plugins.orphaned_payload, 0);
    assert_eq!(host.app.plugins.provider_reload_cleanup, 0);
    assert_eq!(
        host.app.plugins.instances[&1].application.retained_payload,
        0
    );
    assert!(host.app.host_buffer_is_closed(index));
    assert!(
        host.app.buffers[index]
            .provider()
            .is_none_or(|document| document.uncertain.is_none())
    );
}
