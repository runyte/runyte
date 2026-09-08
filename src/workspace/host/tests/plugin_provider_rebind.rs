// SPDX-License-Identifier: MPL-2.0

use super::filesystem::response;
use super::provider_writes::{document, finished, save, upload};
use super::providers::{metadata, opened, reply, resource_request};
use super::*;
use crate::plugin::provider as wire;

fn rebind(
    host: &mut WorkspaceHost,
    requester: &mut mpsc::Receiver<HostMessage>,
    handle: &str,
    index: usize,
    serial: u64,
) -> String {
    request(
        host,
        0,
        serial,
        api::Request::ResourceRebind {
            buffer: handle.into(),
            expected_revision: format!("r:{}", host.app.buffers[index].revision()),
        },
    );
    let active = job(requester);
    assert!(matches!(
        next(requester),
        api::HostMessage::Event {
            event: "job.changed",
            ..
        }
    ));
    assert!(host.app.document_mutation_pending(index));
    active.job
}

fn read_remote(host: &mut WorkspaceHost, provider: &mut mpsc::Receiver<HostMessage>, text: &str) {
    let (id, request) = resource_request(provider);
    assert!(
        matches!(request, wire::Request::Read { ref version, offset: 0, .. } if version == "version-2")
    );
    reply(
        host,
        id,
        wire::Response::Read(wire::Chunk {
            version: "version-2".into(),
            offset: 0,
            text: text.into(),
            eof: true,
        }),
    );
}

fn new_metadata(text: &str) -> wire::Metadata {
    let mut value = metadata(text.len());
    value.version = "version-2".into();
    value
}

fn uncertain(
    host: &mut WorkspaceHost,
    requester: &mut mpsc::Receiver<HostMessage>,
    provider: &mut mpsc::Receiver<HostMessage>,
    handle: &str,
    index: usize,
) -> (String, String) {
    host.app.buffers[index].apply(&Transaction::insert(0, "uploaded "));
    let job = save(host, requester, handle, index);
    let begin = resource_request(provider);
    let (commit, _) = upload(host, provider, begin);
    assert!(host.provider_write_deadline(1, &commit).unwrap());
    let result = finished(requester, api::JobState::OutcomeUnknown);
    assert_eq!(result.error.unwrap().code, api::ErrorCode::OutcomeUnknown);
    assert_eq!(
        host.provider_uncertain[&index],
        ("app-0".into(), wire::READ_CHARGE, job.job.clone())
    );
    assert!(!host.app.document_mutation_pending(index));
    (job.job, commit)
}

#[test]
fn provider_rebind_after_restart_reads_baseline_and_preserves_live_edits() {
    let (_root, mut host) = host();
    let (mut requester, _provider, handle, index) = document(&mut host, "base");
    let generation = host.app.buffers[index]
        .provider()
        .unwrap()
        .generation
        .clone();
    host.app.buffers[index].apply(&Transaction::insert(0, "local "));
    host.stop_plugin(1, "restart test");
    assert!(!host.app.buffers[index].provider().unwrap().available);
    let mut provider = setup(&mut host, 1, &["providers"]);
    next(&mut provider);
    request(
        &mut host,
        1,
        1,
        api::Request::ProviderRegister(wire::Registration {
            name: "remote".into(),
            conditional_write: true,
            atomic_replace: true,
        }),
    );
    response(&mut provider).unwrap();
    rebind(&mut host, &mut requester, &handle, index, 2);
    let (id, stat) = resource_request(&mut provider);
    assert!(matches!(stat, wire::Request::Stat { ref key, .. } if key == "canonical/猫.txt"));
    reply(&mut host, id, wire::Response::Stat(new_metadata("base")));
    assert!(host.app.document_mutation_pending(index));
    host.app.buffers[index].apply(&Transaction::insert(0, "newer "));
    read_remote(&mut host, &mut provider, "base");
    let result = opened(&mut requester);
    assert!(result.error.is_none());
    assert_eq!(result.buffer.as_deref(), Some(handle.as_str()));
    let buffer = &host.app.buffers[index];
    assert_eq!(buffer.to_string(), "newer local base");
    assert!(buffer.dirty);
    let bound = buffer.provider().unwrap();
    assert!(bound.available);
    assert_ne!(bound.generation, generation);
    assert_eq!(bound.version, "version-2");
    assert_eq!(bound.baseline_epoch, 1);
    assert!(!host.app.document_mutation_pending(index));
    assert!(host.provider_reads.is_empty());
    assert_eq!(
        host.app.plugins.instances[&0].application.retained_payload,
        0
    );
}

#[test]
fn provider_rebind_settled_unknown_snapshot_releases_requester_charge_and_ignores_late_commit() {
    let (_root, mut host) = host();
    let (mut requester, mut provider, handle, index) = document(&mut host, "base");
    let (previous, commit) = uncertain(&mut host, &mut requester, &mut provider, &handle, index);
    assert!(
        host.reserve_application_payload(0, 48 * 1024 * 1024)
            .is_err()
    );
    assert!(
        host.reserve_application_payload(1, 48 * 1024 * 1024)
            .is_ok()
    );
    host.app.buffers[index].commit_undo_group();
    host.app.buffers[index].apply(&Transaction::insert(0, "later "));
    host.app.buffers[index].commit_undo_group();
    rebind(&mut host, &mut requester, &handle, index, 11);
    let (id, reconcile) = resource_request(&mut provider);
    assert!(
        matches!(reconcile, wire::Request::Reconcile { ref previous_write, .. } if previous_write == &previous)
    );
    reply(
        &mut host,
        id,
        wire::Response::Reconciled {
            metadata: new_metadata("uploaded base"),
            previous_write: previous,
        },
    );
    read_remote(&mut host, &mut provider, "uploaded base");
    assert!(opened(&mut requester).error.is_none());
    host.sync_plugin_observers();
    assert!(host.provider_uncertain.is_empty());
    assert_eq!(host.app.plugins.orphaned_payload, 0);
    assert!(
        host.reserve_application_payload(0, 48 * 1024 * 1024)
            .is_ok()
    );
    assert!(!host.app.document_mutation_pending(index));
    assert_eq!(host.app.buffers[index].to_string(), "later uploaded base");
    assert!(host.app.buffers[index].dirty);
    reply(
        &mut host,
        commit,
        wire::Response::WriteCommitted {
            version: "obsolete".into(),
        },
    );
    assert_eq!(
        host.app.buffers[index].provider().unwrap().version,
        "version-2"
    );
    assert!(host.app.buffers[index].undo());
    assert_eq!(host.app.buffers[index].to_string(), "uploaded base");
    assert!(!host.app.buffers[index].dirty);
}

#[test]
fn provider_rebind_requires_matching_settlement_proof_and_conflicts_can_be_retried() {
    let (_root, mut host) = host();
    let (mut requester, mut provider, handle, index) = document(&mut host, "base");
    let (previous, _) = uncertain(&mut host, &mut requester, &mut provider, &handle, index);
    for (serial, value, expected, text) in [
        (
            11,
            wire::Response::Stat(new_metadata("base")),
            api::ErrorCode::OutcomeUnknown,
            None,
        ),
        (
            12,
            wire::Response::Reconciled {
                metadata: new_metadata("base"),
                previous_write: "unrelated-write".into(),
            },
            api::ErrorCode::OutcomeUnknown,
            None,
        ),
        (
            13,
            wire::Response::Reconciled {
                metadata: new_metadata("diverged"),
                previous_write: previous.clone(),
            },
            api::ErrorCode::Conflict,
            Some("diverged"),
        ),
    ] {
        rebind(&mut host, &mut requester, &handle, index, serial);
        let (id, reconcile) = resource_request(&mut provider);
        assert!(
            matches!(reconcile, wire::Request::Reconcile { ref previous_write, .. } if previous_write == &previous)
        );
        reply(&mut host, id, value);
        if let Some(text) = text {
            read_remote(&mut host, &mut provider, text);
        }
        assert_eq!(opened(&mut requester).error.unwrap().code, expected);
        assert!(!host.app.document_mutation_pending(index));
        assert_eq!(host.app.buffers[index].to_string(), "uploaded base");
        let bound = host.app.buffers[index].provider().unwrap();
        assert_eq!(bound.version, "version-1");
        assert_eq!(bound.baseline_epoch, 0);
        assert!(bound.uncertain.is_some());
        assert_eq!(host.app.plugins.orphaned_payload, wire::READ_CHARGE);
        assert_eq!(
            host.app.plugins.instances[&0].application.retained_payload,
            0
        );
    }
    rebind(&mut host, &mut requester, &handle, index, 14);
    let (id, _) = resource_request(&mut provider);
    reply(
        &mut host,
        id,
        wire::Response::Reconciled {
            metadata: new_metadata("base"),
            previous_write: previous,
        },
    );
    read_remote(&mut host, &mut provider, "base");
    assert!(opened(&mut requester).error.is_none());
    host.sync_plugin_observers();
    assert_eq!(host.app.buffers[index].to_string(), "uploaded base");
    assert!(host.app.buffers[index].dirty);
    assert!(
        host.app.buffers[index]
            .provider()
            .unwrap()
            .uncertain
            .is_none()
    );
    assert_eq!(host.app.plugins.orphaned_payload, 0);
}

#[test]
fn provider_rebind_guards_mutations_and_timeout_releases_protection() {
    let (_root, mut host) = host();
    let (mut requester, mut provider, handle, index) = document(&mut host, "base");
    let pending = rebind(&mut host, &mut requester, &handle, index, 2);
    let (id, _) = resource_request(&mut provider);
    let revision = format!("r:{}", host.app.buffers[index].revision());
    for (serial, value) in [
        (
            3,
            api::Request::BufferSave {
                buffer: handle.clone(),
                expected_revision: revision.clone(),
            },
        ),
        (
            4,
            api::Request::ResourceRebind {
                buffer: handle.clone(),
                expected_revision: revision,
            },
        ),
    ] {
        request(&mut host, 0, serial, value);
        assert_eq!(
            response(&mut requester).unwrap_err().code,
            api::ErrorCode::Busy
        );
    }
    assert!(host.app.host_close_buffer(index, true).is_err());
    assert!(host.provider_deadline(0, &pending));
    assert_eq!(
        opened(&mut requester).error.unwrap().code,
        api::ErrorCode::Timeout
    );
    assert!(!host.app.document_mutation_pending(index));
    assert!(!host.provider_deadline(0, &pending));
    reply(&mut host, id, wire::Response::Stat(new_metadata("base")));
    assert_eq!(
        host.app.buffers[index].provider().unwrap().version,
        "version-1"
    );
    assert!(host.app.plugins.instances.contains_key(&1));
}

#[test]
fn provider_rebind_rejects_baseline_changed_while_chunk_read_was_pending() {
    let (_root, mut host) = host();
    let (mut requester, mut provider, handle, index) = document(&mut host, "base");
    rebind(&mut host, &mut requester, &handle, index, 2);
    let (id, _) = resource_request(&mut provider);
    reply(&mut host, id, wire::Response::Stat(new_metadata("base")));
    let identity = host.app.buffers[index].provider().unwrap().identity.clone();
    host.app.buffers[index]
        .reconcile_provider(
            &identity,
            "new-generation".into(),
            "new-version".into(),
            "base",
        )
        .unwrap();
    read_remote(&mut host, &mut provider, "base");
    assert_eq!(
        opened(&mut requester).error.unwrap().code,
        api::ErrorCode::Conflict
    );
    let bound = host.app.buffers[index].provider().unwrap();
    assert_eq!(bound.version, "new-version");
    assert_eq!(bound.generation, "new-generation");
    assert_eq!(bound.baseline_epoch, 1);
    assert!(!host.app.document_mutation_pending(index));
}

#[test]
fn provider_rebind_stop_and_cancel_release_guards_without_resolving_unknown_writes() {
    for stopped in [Some(0), Some(1), None] {
        let (_root, mut host) = host();
        let (mut requester, mut provider, handle, index) = document(&mut host, "base");
        let (previous, _) = uncertain(&mut host, &mut requester, &mut provider, &handle, index);
        let pending = rebind(&mut host, &mut requester, &handle, index, 11);
        let (id, _) = resource_request(&mut provider);
        if let Some(owner) = stopped {
            host.stop_plugin(owner, "rebind stop regression");
        } else {
            host.provider_job_request(0, &api::Request::JobCancel { job: pending })
                .unwrap()
                .unwrap();
        }
        if stopped != Some(0) {
            assert_eq!(
                opened(&mut requester).error.unwrap().code,
                if stopped.is_none() {
                    api::ErrorCode::Cancelled
                } else {
                    api::ErrorCode::Unavailable
                }
            );
            assert_eq!(
                host.app.plugins.instances[&0].application.retained_payload,
                0
            );
        }
        assert!(host.provider_reads.is_empty());
        assert!(!host.app.document_mutation_pending(index));
        assert_eq!(
            host.provider_uncertain[&index],
            ("app-0".into(), wire::READ_CHARGE, previous)
        );
        assert_eq!(host.app.plugins.orphaned_payload, wire::READ_CHARGE);
        assert!(
            host.app.buffers[index]
                .provider()
                .unwrap()
                .uncertain
                .is_some()
        );
        assert!(host.app.buffers[index].dirty);
        if stopped != Some(1) {
            reply(&mut host, id, wire::Response::Stat(new_metadata("base")));
            assert!(host.app.plugins.instances.contains_key(&1));
            assert_eq!(
                host.app.plugins.instances[&1].application.provider_requests,
                0
            );
        }
    }
}

#[test]
fn provider_rebind_settlement_read_rejects_version_changes_without_clearing_uncertainty() {
    let (_root, mut host) = host();
    let (mut requester, mut provider, handle, index) = document(&mut host, "base");
    let (previous, _) = uncertain(&mut host, &mut requester, &mut provider, &handle, index);
    rebind(&mut host, &mut requester, &handle, index, 11);
    let (id, _) = resource_request(&mut provider);
    reply(
        &mut host,
        id,
        wire::Response::Reconciled {
            metadata: new_metadata("base"),
            previous_write: previous,
        },
    );
    let (id, _) = resource_request(&mut provider);
    reply(
        &mut host,
        id,
        wire::Response::Read(wire::Chunk {
            version: "version-3".into(),
            offset: 0,
            text: "base".into(),
            eof: true,
        }),
    );
    assert_eq!(
        opened(&mut requester).error.unwrap().code,
        api::ErrorCode::Stale
    );
    assert!(!host.app.document_mutation_pending(index));
    assert!(
        host.app.buffers[index]
            .provider()
            .unwrap()
            .uncertain
            .is_some()
    );
    assert_eq!(
        host.app.buffers[index].provider().unwrap().version,
        "version-1"
    );
    assert_eq!(host.app.plugins.orphaned_payload, wire::READ_CHARGE);
}

#[test]
fn provider_rebind_unknown_write_reuses_reservation_at_owner_and_global_quota() {
    let (_root, mut host) = host();
    let (mut requester, mut provider, handle, index) = document(&mut host, "base");
    let (previous, _) = uncertain(&mut host, &mut requester, &mut provider, &handle, index);
    // Fill remaining quota with other retained host models. Recovery needs to
    // coexist with them, including when another application owns the payload.
    host.app
        .plugins
        .instances
        .get_mut(&0)
        .unwrap()
        .application
        .retained_payload = 32 * 1024 * 1024;
    let mut others = Vec::new();
    for (owner, bytes) in [(2, 48), (3, 48), (4, 16)] {
        let mut output = setup(&mut host, owner, &[]);
        next(&mut output);
        host.app
            .plugins
            .instances
            .get_mut(&owner)
            .unwrap()
            .application
            .retained_payload = bytes * 1024 * 1024;
        others.push(output);
    }
    assert!(host.reserve_application_payload(0, 0).is_ok());
    assert!(host.reserve_application_payload(0, 1).is_err());
    assert!(host.reserve_application_payload(1, 1).is_err());
    rebind(&mut host, &mut requester, &handle, index, 11);
    assert_eq!(
        host.app.plugins.instances[&0].application.retained_payload,
        32 * 1024 * 1024
    );
    let (id, _) = resource_request(&mut provider);
    reply(
        &mut host,
        id,
        wire::Response::Reconciled {
            metadata: new_metadata("uploaded base"),
            previous_write: previous,
        },
    );
    read_remote(&mut host, &mut provider, "uploaded base");
    assert!(opened(&mut requester).error.is_none());
    assert!(host.provider_uncertain.is_empty());
    assert_eq!(host.app.plugins.orphaned_payload, 0);
    assert_eq!(
        host.app.plugins.instances[&0].application.retained_payload,
        32 * 1024 * 1024
    );
    assert!(
        host.reserve_application_payload(0, wire::READ_CHARGE)
            .is_ok()
    );
    assert!(!host.app.document_mutation_pending(index));
    assert!(!host.app.buffers[index].dirty);
}
