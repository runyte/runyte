// SPDX-License-Identifier: MPL-2.0

use super::filesystem::response;
use super::provider_writes::document;
use super::providers::{metadata, reply, resource_request};
use super::*;
use crate::plugin::provider as wire;

fn invocation(
    host: &mut WorkspaceHost,
    output: &mut mpsc::Receiver<HostMessage>,
    index: usize,
) -> String {
    host.app.show_provider_document(index);
    host.app.note_plugin_frontend(true);
    invoke(host, "plugin.app-0.open");
    let api::HostMessage::Request { id, .. } = next(output) else {
        panic!()
    };
    id
}

fn inspect(
    host: &mut WorkspaceHost,
    output: &mut mpsc::Receiver<HostMessage>,
    handle: &str,
    index: usize,
) -> String {
    let invocation = invocation(host, output, index);
    let serial = host.app.plugins.instances[&0].application.last_request + 1;
    request(
        host,
        0,
        serial,
        api::Request::ResourceInspect {
            buffer: handle.into(),
            expected_revision: format!("r:{}", host.app.buffers[index].revision()),
            invocation,
        },
    );
    let active = job(output);
    assert!(matches!(
        next(output),
        api::HostMessage::Event {
            event: "job.changed",
            ..
        }
    ));
    assert!(!host.app.document_mutation_pending(index));
    active.job
}

fn inspected(output: &mut mpsc::Receiver<HostMessage>) -> wire::Finished {
    let api::HostMessage::Event {
        event: "resource.inspected",
        data: api::EventData::ResourceFinished(value),
        ..
    } = next(output)
    else {
        panic!()
    };
    let api::HostMessage::Event {
        event: "job.changed",
        data: api::EventData::Job(job),
        ..
    } = next(output)
    else {
        panic!()
    };
    assert!(!job.state.active());
    value
}

fn remote(host: &mut WorkspaceHost, provider: &mut mpsc::Receiver<HostMessage>, text: &str) {
    let (id, req) = resource_request(provider);
    assert!(matches!(req, wire::Request::Stat { .. }));
    let mut metadata = metadata(text.len());
    metadata.version = "remote-version".into();
    reply(host, id, wire::Response::Stat(metadata));
    let (id, req) = resource_request(provider);
    assert!(matches!(req, wire::Request::Read { .. }));
    reply(
        host,
        id,
        wire::Response::Read(wire::Chunk {
            version: "remote-version".into(),
            offset: 0,
            text: text.into(),
            eof: true,
        }),
    );
}

#[test]
fn provider_inspect_compares_fresh_remote_text_without_adopting_a_baseline() {
    let (_root, mut host) = host();
    let (mut requester, mut provider, handle, index) = document(&mut host, "base");
    host.app.buffers[index].apply(&Transaction::insert(0, "local "));
    let original = host.app.buffers[index].provider().unwrap().clone();
    inspect(&mut host, &mut requester, &handle, index);
    remote(&mut host, &mut provider, "remote 猫\r\n");
    let result = inspected(&mut requester);
    assert!(result.error.is_none());
    let snapshot = host.app.plugins.instances[&0].application.buffers[&result.buffer.unwrap()];
    assert_ne!(snapshot, index);
    assert_eq!(host.app.buffers[snapshot].to_string(), "remote 猫\r\n");
    assert!(host.app.buffers[snapshot].is_read_only());
    assert!(host.app.buffers[snapshot].path.is_none());
    assert_eq!(host.app.buffers[index].to_string(), "local base");
    assert!(host.app.buffers[index].dirty);
    let current = host.app.buffers[index].provider().unwrap();
    assert_eq!(current.version, original.version);
    assert_eq!(current.baseline_epoch, original.baseline_epoch);
    assert!(!host.app.diffs.is_empty());
    assert_eq!(
        host.app.plugins.instances[&0].application.retained_payload,
        0
    );
}

#[test]
fn provider_inspect_refuses_missing_invocation_stale_revision_and_wrong_document() {
    for mode in ["missing", "stale", "target", "capability", "terminal"] {
        let (_root, mut host) = host();
        let (mut requester, _provider, handle, index) = document(&mut host, "base");
        let mut invocation = invocation(&mut host, &mut requester, index);
        let mut revision = format!("r:{}", host.app.buffers[index].revision());
        let expected = match mode {
            "missing" => {
                invocation = "forged".into();
                api::ErrorCode::ContextChanged
            }
            "stale" => {
                revision = "r:99999".into();
                api::ErrorCode::Stale
            }
            "target" => {
                host.app
                    .plugins
                    .instances
                    .get_mut(&0)
                    .unwrap()
                    .application
                    .requests
                    .get_mut(&invocation)
                    .unwrap()
                    .buffer = 0;
                api::ErrorCode::ContextChanged
            }
            "terminal" => {
                let terminal = Some(crate::terminal::TerminalId::from_raw(999));
                host.app
                    .panes
                    .get_mut(&host.app.active_pane)
                    .unwrap()
                    .terminal = terminal;
                host.app
                    .plugins
                    .instances
                    .get_mut(&0)
                    .unwrap()
                    .application
                    .requests
                    .get_mut(&invocation)
                    .unwrap()
                    .terminal = terminal;
                api::ErrorCode::ContextChanged
            }
            _ => {
                host.app
                    .plugins
                    .instances
                    .get_mut(&0)
                    .unwrap()
                    .application
                    .capabilities
                    .remove("jobs");
                api::ErrorCode::CapabilityDenied
            }
        };
        request(
            &mut host,
            0,
            20,
            api::Request::ResourceInspect {
                buffer: handle,
                expected_revision: revision,
                invocation,
            },
        );
        assert_eq!(
            response(&mut requester).unwrap_err().code,
            expected,
            "{mode}"
        );
        assert!(host.provider_reads.is_empty());
        assert_eq!(
            host.app.plugins.instances[&0].application.retained_payload,
            0
        );
    }
}

#[test]
fn provider_inspect_rejects_stale_publication_without_changing_panes_or_retaining_text() {
    for mode in ["edit", "input", "detach", "baseline"] {
        let (_root, mut host) = host();
        let (mut requester, mut provider, handle, index) = document(&mut host, "base");
        inspect(&mut host, &mut requester, &handle, index);
        match mode {
            "edit" => {
                host.app.buffers[index].apply(&Transaction::insert(0, "new "));
            }
            "input" => host
                .app
                .handle_key(crate::input::KeyStroke::char('l'))
                .unwrap(),
            "detach" => host.app.note_plugin_frontend(false),
            _ => {
                host.app.buffers[index]
                    .provider_mut()
                    .unwrap()
                    .baseline_epoch += 1
            }
        }
        let count = host.app.buffers.len();
        let active = host.app.active().buffer;
        remote(&mut host, &mut provider, "remote");
        let result = inspected(&mut requester);
        assert!(result.error.is_some(), "{mode}");
        assert!(result.buffer.is_none());
        assert_eq!(host.app.buffers.len(), count);
        assert_eq!(host.app.active().buffer, active);
        assert!(host.app.diffs.is_empty());
        assert_eq!(
            host.app.plugins.instances[&0].application.retained_payload,
            0
        );
    }
}

#[test]
fn provider_inspect_limits_metadata_and_retires_cancelled_calls() {
    for mode in ["size", "identity", "cancel", "timeout", "stop"] {
        let (_root, mut host) = host();
        let (mut requester, mut provider, handle, index) = document(&mut host, "base");
        let job = inspect(&mut host, &mut requester, &handle, index);
        let (id, _) = resource_request(&mut provider);
        match mode {
            "size" => reply(
                &mut host,
                id,
                wire::Response::Stat(metadata(crate::diff_view::MAX_DIFF_BYTES + 1)),
            ),
            "identity" => {
                let mut value = metadata(0);
                value.key = "other".into();
                reply(&mut host, id, wire::Response::Stat(value));
            }
            "cancel" => {
                host.provider_job_request(0, &api::Request::JobCancel { job })
                    .unwrap()
                    .unwrap();
                reply(&mut host, id, wire::Response::Stat(metadata(0)));
            }
            "timeout" => {
                assert!(host.provider_deadline(1, &id));
                reply(&mut host, id, wire::Response::Stat(metadata(0)));
            }
            _ => host.stop_plugin(1, "test inspection provider stop"),
        }
        assert!(inspected(&mut requester).error.is_some(), "{mode}");
        assert!(host.provider_reads.is_empty());
        assert_eq!(
            host.app.plugins.instances[&0].application.retained_payload,
            0
        );
        assert!(host.app.diffs.is_empty());
    }
}

#[test]
fn provider_inspect_native_command_uses_provider_without_jobs_grant() {
    let (_root, mut host) = host();
    let (_requester, mut provider, _handle, index) = document(&mut host, "base");
    host.app.show_provider_document(index);
    host.app.note_plugin_frontend(true);
    invoke(&mut host, "diff-remote");
    host.sync_provider_inspections();
    // Initial resource request is enqueued before the native job event.
    let (id, _) = resource_request(&mut provider);
    assert!(matches!(
        next(&mut provider),
        api::HostMessage::Event {
            event: "job.changed",
            ..
        }
    ));
    reply(&mut host, id, wire::Response::Stat(metadata(0)));
    let (id, _) = resource_request(&mut provider);
    reply(
        &mut host,
        id,
        wire::Response::Read(wire::Chunk {
            version: "version-1".into(),
            offset: 0,
            text: String::new(),
            eof: true,
        }),
    );
    assert!(inspected(&mut provider).error.is_none());
    assert!(!host.app.diffs.is_empty());
    assert!(
        !host.app.plugins.instances[&1]
            .application
            .capabilities
            .contains("jobs")
    );
}

#[test]
fn provider_inspect_reuses_owned_snapshot_at_handle_capacity_but_refuses_new_publication() {
    for existing in [false, true] {
        let (_root, mut host) = host();
        let (mut requester, mut provider, handle, index) = document(&mut host, "base");
        let snapshot = if existing {
            inspect(&mut host, &mut requester, &handle, index);
            remote(&mut host, &mut provider, "first");
            Some(inspected(&mut requester).buffer.unwrap())
        } else {
            None
        };
        inspect(&mut host, &mut requester, &handle, index);
        let state = &mut host.app.plugins.instances.get_mut(&0).unwrap().application;
        while state.buffers.len() < 1024 {
            state
                .buffers
                .insert(format!("filler:{}", state.buffers.len()), 0);
        }
        let count = host.app.buffers.len();
        remote(&mut host, &mut provider, "second");
        let result = inspected(&mut requester);
        if existing {
            assert_eq!(result.buffer, snapshot);
            assert!(result.error.is_none());
            let buffer =
                host.app.plugins.instances[&0].application.buffers[&result.buffer.unwrap()];
            assert_eq!(host.app.buffers[buffer].to_string(), "second");
        } else {
            assert_eq!(result.error.unwrap().code, api::ErrorCode::LimitExceeded);
            assert!(host.app.diffs.is_empty());
        }
        assert_eq!(host.app.buffers.len(), count);
        assert_eq!(
            host.app.plugins.instances[&0].application.retained_payload,
            0
        );
    }
}
