// SPDX-License-Identifier: MPL-2.0

use super::providers::{chunk, metadata, open, opened, pair, reply, resource_request};
use super::*;
use crate::plugin::provider as wire;

fn released(receiver: &mut mpsc::Receiver<HostMessage>, job: &str) {
    let mut releases = Vec::new();
    while let Ok(message) = receiver.try_recv() {
        match message {
            HostMessage::Deadline { .. } => {}
            HostMessage::Application(api::HostMessage::Event {
                event: "resource.released",
                data: api::EventData::ResourceReleased { job },
                ..
            }) => releases.push(job),
            other => panic!("unexpected provider output {other:?}"),
        }
    }
    assert_eq!(releases, vec![job]);
}

#[test]
fn provider_read_terminal_paths_release_the_actual_providers_cache_once() {
    for mode in ["success", "cancel", "timeout", "metadata", "requester-stop"] {
        let (_root, mut host) = host();
        let (mut requester, mut provider) = pair(&mut host);
        let job = open(&mut host, &mut requester, 1, None);
        let (id, _) = resource_request(&mut provider);
        match mode {
            "success" => {
                reply(&mut host, id, wire::Response::Stat(metadata(4)));
                let (read, _) = resource_request(&mut provider);
                chunk(&mut host, read, 0, "text", true);
            }
            "cancel" => {
                host.provider_job_request(0, &api::Request::JobCancel { job: job.clone() })
                    .unwrap()
                    .unwrap();
            }
            "timeout" => {
                assert!(host.provider_deadline(0, &job));
            }
            "metadata" => {
                let mut value = metadata(0);
                value.encoding = "binary".into();
                reply(&mut host, id, wire::Response::Stat(value));
            }
            _ => host.stop_plugin(0, "requester stopped"),
        }
        released(&mut provider, &job);
        assert!(host.app.plugins.instances.contains_key(&1));
        assert!(host.provider_reads.is_empty());
        if mode != "requester-stop" {
            assert_eq!(opened(&mut requester).job, job);
        }
    }
}

#[test]
fn provider_live_identity_shortcut_releases_the_fresh_stat_snapshot() {
    let (_root, mut host) = host();
    let (mut requester, mut provider) = pair(&mut host);
    let first = open(&mut host, &mut requester, 1, None);
    let (id, _) = resource_request(&mut provider);
    reply(&mut host, id, wire::Response::Stat(metadata(0)));
    let (id, _) = resource_request(&mut provider);
    chunk(&mut host, id, 0, "", true);
    let first_buffer = opened(&mut requester).buffer;
    released(&mut provider, &first);
    let second = open(&mut host, &mut requester, 2, None);
    let (id, _) = resource_request(&mut provider);
    reply(&mut host, id, wire::Response::Stat(metadata(100)));
    assert_eq!(opened(&mut requester).buffer, first_buffer);
    released(&mut provider, &second);
}

#[test]
fn provider_cache_release_delivery_failure_stops_owner_and_settles_requester() {
    let (_root, mut host) = host();
    let (mut requester, mut provider) = pair(&mut host);
    let job = open(&mut host, &mut requester, 1, None);
    resource_request(&mut provider);
    drop(provider);
    assert!(host.provider_deadline(0, &job));
    assert!(!host.app.plugins.instances.contains_key(&1));
    assert!(host.provider_reads.is_empty());
    assert_eq!(
        opened(&mut requester).error.unwrap().code,
        api::ErrorCode::Timeout
    );
    assert_eq!(
        host.app.plugins.instances[&0].application.retained_payload,
        0
    );
}
