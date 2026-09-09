// SPDX-License-Identifier: MPL-2.0

use super::filesystem::response;
use super::providers::{chunk, metadata, open, opened, pair, reply, resource_request, stat};
use super::*;
use crate::plugin::provider as wire;

fn fill_handles(host: &mut WorkspaceHost) {
    while host.app.plugins.instances[&0].application.buffers.len() < 1024 {
        let index = host.app.buffers.len();
        host.app.buffers.push(crate::buffer::Buffer::scratch());
        host.app.syntax.push(None);
        host.app
            .plugins
            .instances
            .get_mut(&0)
            .unwrap()
            .application
            .buffer_handle(index)
            .unwrap();
    }
}

fn fill_queue(
    host: &WorkspaceHost,
    owner: usize,
    output: &mut mpsc::Receiver<HostMessage>,
    free: usize,
) {
    let sender = &host.app.plugins.instances[&owner].sender;
    while sender
        .try_send(HostMessage::Deadline {
            token: "queue-filler".into(),
            after_ms: None,
        })
        .is_ok()
    {}
    for _ in 0..free {
        output.try_recv().unwrap();
    }
}

#[test]
fn provider_read_handle_exhaustion_refuses_before_publishing_a_buffer() {
    let (_root, mut host) = host();
    let (mut requester, mut provider) = pair(&mut host);
    let token = open(&mut host, &mut requester, 1, None);
    let (id, _) = stat(&mut host, &mut provider, 2);
    fill_handles(&mut host);
    let before = host.app.buffers.len();
    chunk(&mut host, id, 0, "é", true);
    let result = opened(&mut requester);
    assert_eq!(result.job, token);
    assert_eq!(result.error.unwrap().code, api::ErrorCode::LimitExceeded);
    assert!(result.buffer.is_none());
    assert_eq!(host.app.buffers.len(), before);
    assert!(
        host.app
            .buffers
            .iter()
            .all(|buffer| buffer.provider().is_none())
    );
    assert!(host.provider_reads.is_empty());
    let state = &host.app.plugins.instances[&0].application;
    assert_eq!(state.retained_payload, 0);
    assert!(state.deadlines.is_empty());
    assert_eq!(state.jobs[&token].state, api::JobState::Failed);
}

#[test]
fn provider_open_reuses_an_existing_issued_handle_at_capacity() {
    let (_root, mut host) = host();
    let (mut requester, mut provider) = pair(&mut host);
    open(&mut host, &mut requester, 1, None);
    let (id, _) = stat(&mut host, &mut provider, 2);
    chunk(&mut host, id, 0, "é", true);
    let handle = opened(&mut requester).buffer.unwrap();
    let index = host.app.plugins.instances[&0].application.buffers[&handle];
    host.app.buffers[index].apply(&Transaction::insert(0, "edited "));
    assert!(host.save_buffer(BufferId::from_index(index)).is_err());
    assert!(host.app.buffers[index].dirty);
    assert!(host.app.buffers[index].path.is_none());
    fill_handles(&mut host);
    let before = host.app.buffers.len();
    open(&mut host, &mut requester, 2, None);
    let (id, _) = resource_request(&mut provider);
    reply(&mut host, id, wire::Response::Stat(metadata(2)));
    let result = opened(&mut requester);
    assert!(result.error.is_none());
    assert_eq!(result.buffer.as_ref(), Some(&handle));
    assert_eq!(host.app.buffers.len(), before);
    assert_eq!(host.app.buffers[index].to_string(), "edited é");
    assert!(host.app.buffers[index].dirty);
    assert!(host.provider_reads.is_empty());
    assert_eq!(
        host.app.plugins.instances[&0].application.retained_payload,
        0
    );
}

#[test]
fn provider_initial_dispatch_failure_quietly_rolls_back_unannounced_work() {
    for free in [0, 1] {
        let (_root, mut host) = host();
        let (mut requester, mut provider) = pair(&mut host);
        fill_queue(&host, 1, &mut provider, free);
        request(
            &mut host,
            0,
            1,
            api::Request::ResourceOpen {
                plugin: "app-1".into(),
                provider: "remote".into(),
                key: "file".into(),
                invocation: None,
            },
        );
        assert_eq!(
            response(&mut requester).unwrap_err().code,
            api::ErrorCode::Unavailable
        );
        while let Ok(message) = requester.try_recv() {
            assert!(
                matches!(message, HostMessage::Deadline { .. }),
                "unannounced result: {message:?}"
            );
        }
        assert!(!host.app.plugins.instances.contains_key(&1));
        let state = &host.app.plugins.instances[&0].application;
        assert_eq!(state.retained_payload, 0);
        assert!(state.jobs.is_empty());
        assert!(state.finished_jobs.is_empty());
        assert!(state.deadlines.is_empty());
        assert!(host.provider_reads.is_empty());
        assert_eq!(host.protected_state().plugin_jobs, 0);
    }
}

#[test]
fn provider_initial_job_deadline_failure_stops_requester_without_leaking_work() {
    let (_root, mut host) = host();
    let (mut requester, mut provider) = pair(&mut host);
    fill_queue(&host, 0, &mut requester, 0);
    request(
        &mut host,
        0,
        1,
        api::Request::ResourceOpen {
            plugin: "app-1".into(),
            provider: "remote".into(),
            key: "file".into(),
            invocation: None,
        },
    );
    assert!(!host.app.plugins.instances.contains_key(&0));
    assert!(host.app.plugins.instances.contains_key(&1));
    assert!(host.provider_reads.is_empty());
    assert_eq!(host.protected_state().plugin_jobs, 0);
    assert_eq!(host.app.plugins.orphaned_payload, 0);
    assert!(provider.try_recv().is_err());
    while let Ok(message) = requester.try_recv() {
        assert!(matches!(message, HostMessage::Deadline { .. }));
    }
}

#[test]
fn provider_mid_read_dispatch_failure_cleans_payload_and_notifies_requester_once() {
    let (_root, mut host) = host();
    let (mut requester, mut provider) = pair(&mut host);
    let token = open(&mut host, &mut requester, 1, None);
    let (id, _) = stat(&mut host, &mut provider, 4);
    let before = host.app.buffers.len();
    // Clearing the answered request and arming the next deadline fit, but the
    // next actual resource.read request exhausts the provider queue.
    fill_queue(&host, 1, &mut provider, 2);
    host.handle_plugin_event(Event {
        plugin: 1,
        result: Ok(ClientMessage::Application(api::ClientMessage::Response {
            id,
            outcome: api::CommandResponse::Resource {
                result: wire::Response::Read(wire::Chunk {
                    version: "version-1".into(),
                    offset: 0,
                    text: "é".into(),
                    eof: false,
                }),
            },
        })),
    });
    let result = opened(&mut requester);
    assert_eq!(result.job, token);
    assert_eq!(result.error.unwrap().code, api::ErrorCode::Unavailable);
    assert!(result.buffer.is_none());
    assert!(!host.app.plugins.instances.contains_key(&1));
    assert_eq!(host.app.buffers.len(), before);
    assert!(host.provider_reads.is_empty());
    let state = &host.app.plugins.instances[&0].application;
    assert_eq!(state.retained_payload, 0);
    assert!(state.deadlines.is_empty());
    assert_eq!(state.jobs[&token].state, api::JobState::Failed);
    assert_eq!(host.protected_state().plugin_jobs, 0);
    assert!(requester.try_recv().is_err());
}

#[test]
fn provider_calls_and_commands_share_the_control_request_limit() {
    let (_root, mut host) = host();
    let (mut requester, mut provider) = pair(&mut host);
    open(&mut host, &mut requester, 1, None);
    let (stat_id, _) = resource_request(&mut provider);
    assert_eq!(
        host.app.plugins.instances[&1].application.provider_requests,
        1
    );
    for _ in 1..api::MAX_REQUESTS {
        invoke(&mut host, "plugin.app-1.open");
        assert!(matches!(
            next(&mut provider),
            api::HostMessage::Request { .. }
        ));
    }
    invoke(&mut host, "plugin.app-1.open");
    while let Ok(message) = provider.try_recv() {
        assert!(
            matches!(message, HostMessage::Deadline { .. }),
            "excess control request: {message:?}"
        );
    }
    assert_eq!(
        host.app.plugins.instances[&1].application.requests.len(),
        api::MAX_REQUESTS - 1
    );
    request(
        &mut host,
        0,
        2,
        api::Request::ResourceOpen {
            plugin: "app-1".into(),
            provider: "remote".into(),
            key: "another-resource".into(),
            invocation: None,
        },
    );
    assert_eq!(
        response(&mut requester).unwrap_err().code,
        api::ErrorCode::Busy
    );
    assert_eq!(host.provider_reads.len(), 1);

    // Replacing the stat call with a read call retains one slot, and terminal
    // completion releases that slot for the next ordinary command.
    reply(&mut host, stat_id, wire::Response::Stat(metadata(2)));
    let (read_id, _) = resource_request(&mut provider);
    assert_eq!(
        host.app.plugins.instances[&1].application.provider_requests,
        1
    );
    chunk(&mut host, read_id, 0, "é", true);
    assert!(opened(&mut requester).error.is_none());
    assert_eq!(
        host.app.plugins.instances[&1].application.provider_requests,
        0
    );
    invoke(&mut host, "plugin.app-1.open");
    assert!(matches!(
        next(&mut provider),
        api::HostMessage::Request { .. }
    ));
    assert_eq!(
        host.app.plugins.instances[&1].application.requests.len(),
        api::MAX_REQUESTS
    );
    assert!(host.provider_reads.is_empty());
    assert_eq!(
        host.app.plugins.instances[&0].application.retained_payload,
        0
    );
}
