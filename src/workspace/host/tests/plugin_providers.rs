// SPDX-License-Identifier: MPL-2.0

use super::filesystem::response;
use super::*;
use crate::input::KeyStroke;
use crate::plugin::provider as wire;

pub(super) fn pair(
    host: &mut WorkspaceHost,
) -> (mpsc::Receiver<HostMessage>, mpsc::Receiver<HostMessage>) {
    let mut requester = setup(host, 0, &["documents", "jobs", "text"]);
    let mut provider = setup(host, 1, &["providers"]);
    next(&mut requester);
    next(&mut provider);
    request(
        host,
        1,
        1,
        api::Request::ProviderRegister(wire::Registration {
            name: "remote".into(),
            conditional_write: true,
            atomic_replace: true,
        }),
    );
    response(&mut provider).unwrap();
    (requester, provider)
}
pub(super) fn open(
    host: &mut WorkspaceHost,
    output: &mut mpsc::Receiver<HostMessage>,
    serial: u64,
    invocation: Option<String>,
) -> String {
    open_key(host, output, serial, "requested/猫.txt", invocation)
}
pub(super) fn open_key(
    host: &mut WorkspaceHost,
    output: &mut mpsc::Receiver<HostMessage>,
    serial: u64,
    key: &str,
    invocation: Option<String>,
) -> String {
    request(
        host,
        0,
        serial,
        api::Request::ResourceOpen {
            plugin: "app-1".into(),
            provider: "remote".into(),
            key: key.into(),
            invocation,
        },
    );
    let api::ResultValue::Job(job) = response(output).unwrap() else {
        panic!()
    };
    assert!(
        matches!(next(output), api::HostMessage::Event { event: "job.changed", data: api::EventData::Job(active), .. } if active.job == job.job && active.state.active())
    );
    job.job
}
pub(super) fn resource_request(
    output: &mut mpsc::Receiver<HostMessage>,
) -> (String, wire::Request) {
    match next(output) {
        api::HostMessage::ResourceRequest { id, request } => (id, request),
        other => panic!("unexpected provider message {other:?}"),
    }
}
pub(super) fn metadata(bytes: usize) -> wire::Metadata {
    wire::Metadata {
        key: "canonical/猫.txt".into(),
        label: "Remote 猫.txt".into(),
        syntax_hint: Some("text".into()),
        version: "version-1".into(),
        encoding: "utf-8".into(),
        bytes,
    }
}
pub(super) fn reply(host: &mut WorkspaceHost, id: String, value: wire::Response) {
    host.application_message(
        1,
        api::ClientMessage::Response {
            id,
            outcome: api::CommandResponse::Resource { result: value },
        },
    )
    .unwrap();
}
pub(super) fn stat(
    host: &mut WorkspaceHost,
    output: &mut mpsc::Receiver<HostMessage>,
    bytes: usize,
) -> (String, wire::Request) {
    let (id, request) = resource_request(output);
    assert!(
        matches!(request, wire::Request::Stat { ref provider, ref key, .. } if provider == "remote" && key == "requested/猫.txt")
    );
    reply(host, id, wire::Response::Stat(metadata(bytes)));
    resource_request(output)
}
pub(super) fn chunk(host: &mut WorkspaceHost, id: String, offset: usize, text: &str, eof: bool) {
    reply(
        host,
        id,
        wire::Response::Read(wire::Chunk {
            version: "version-1".into(),
            offset,
            text: text.into(),
            eof,
        }),
    );
}
pub(super) fn opened(output: &mut mpsc::Receiver<HostMessage>) -> wire::Finished {
    let first = next(output);
    let second = next(output);
    let mut finished = None;
    let mut terminal = None;
    for message in [first, second] {
        match message {
            api::HostMessage::Event {
                event: "resource.opened",
                data: api::EventData::ResourceFinished(result),
                ..
            } => finished = Some(result),
            api::HostMessage::Event {
                event: "job.changed",
                data: api::EventData::Job(job),
                ..
            } => terminal = Some(job),
            other => panic!("unexpected completion {other:?}"),
        }
    }
    let finished = finished.unwrap();
    let terminal = terminal.unwrap();
    assert_eq!(terminal.job, finished.job);
    assert!(!terminal.state.active());
    assert_eq!(
        terminal.state == api::JobState::Succeeded,
        finished.error.is_none()
    );
    finished
}
fn edit(host: &mut WorkspaceHost, index: usize, text: &str) {
    host.apply_expected_transaction(
        BufferId::from_index(index),
        BufferRevision::from_raw(host.app.buffers[index].revision()),
        Transaction::insert(0, text),
    )
    .unwrap();
}

#[test]
fn provider_multichunk_unicode_is_byte_and_version_bound_and_writes_no_local_file() {
    let (root, mut host) = host();
    let before = std::fs::read_dir(root.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect::<std::collections::BTreeSet<_>>();
    let (mut requester, mut provider) = pair(&mut host);
    let token = open(&mut host, &mut requester, 1, None);
    let first = "é猫";
    let last = "\nlast🔑";
    let (id, read) = stat(&mut host, &mut provider, first.len() + last.len());
    assert!(
        matches!(read, wire::Request::Read { ref job, ref key, ref version, offset: 0, limit: wire::CHUNK_BYTES, .. } if job == &token && key == "canonical/猫.txt" && version == "version-1")
    );
    assert_eq!(
        host.app.plugins.instances[&0].application.retained_payload,
        wire::READ_CHARGE
    );
    let unrelated = host.application_message(
        0,
        api::ClientMessage::Response {
            id: id.clone(),
            outcome: api::CommandResponse::Resource {
                result: wire::Response::Read(wire::Chunk {
                    version: "version-1".into(),
                    offset: 0,
                    text: first.into(),
                    eof: false,
                }),
            },
        },
    );
    assert!(unrelated.is_err());
    assert_eq!(host.provider_reads.len(), 1);
    chunk(&mut host, id, 0, first, false);
    let (id, read) = resource_request(&mut provider);
    assert!(
        matches!(read, wire::Request::Read { offset, ref version, .. } if offset == first.len() && version == "version-1")
    );
    chunk(&mut host, id.clone(), first.len(), last, true);
    let result = opened(&mut requester);
    assert_eq!(result.job, token);
    let handle = result.buffer.unwrap();
    let buffer = host.app.plugins.instances[&0].application.buffers[&handle];
    assert_eq!(
        host.app.buffers[buffer].to_string(),
        format!("{first}{last}")
    );
    assert!(host.app.buffers[buffer].path.is_none());
    assert!(!host.app.buffers[buffer].dirty);
    assert_eq!(
        host.app.buffers[buffer].provider().unwrap().identity.key,
        "canonical/猫.txt"
    );
    assert!(
        host.app.plugins.instances[&1]
            .application
            .buffers
            .is_empty()
    );
    assert_eq!(
        host.app.plugins.instances[&0].application.retained_payload,
        0
    );
    assert!(host.provider_reads.is_empty());
    assert!(
        host.application_message(
            1,
            api::ClientMessage::Response {
                id,
                outcome: api::CommandResponse::Resource {
                    result: wire::Response::Read(wire::Chunk {
                        version: "version-1".into(),
                        offset: first.len(),
                        text: last.into(),
                        eof: true
                    })
                }
            }
        )
        .is_err()
    );
    assert_eq!(
        std::fs::read_dir(root.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect::<std::collections::BTreeSet<_>>(),
        before
    );
}

#[test]
fn provider_empty_document_uses_explicit_empty_eof() {
    let (_root, mut host) = host();
    let (mut requester, mut provider) = pair(&mut host);
    open(&mut host, &mut requester, 1, None);
    let (id, _) = stat(&mut host, &mut provider, 0);
    chunk(&mut host, id, 0, "", true);
    let result = opened(&mut requester);
    let index = host.app.plugins.instances[&0].application.buffers[result.buffer.as_ref().unwrap()];
    assert_eq!(host.app.buffers[index].to_string(), "");
    assert!(result.error.is_none());
}

#[test]
fn provider_rejects_malformed_binary_oversize_and_mismatched_chunks_without_publication() {
    for fault in [
        "offset",
        "version",
        "binary",
        "empty",
        "early-eof",
        "missing-eof",
        "too-many-bytes",
        "chunk-limit",
        "wrong-phase",
    ] {
        let (_root, mut host) = host();
        let (mut requester, mut provider) = pair(&mut host);
        open(&mut host, &mut requester, 1, None);
        let size = if fault == "chunk-limit" {
            wire::CHUNK_BYTES + 1
        } else {
            2
        };
        let (id, _) = stat(&mut host, &mut provider, size);
        let count = host.app.buffers.len();
        let mut value = wire::Chunk {
            version: "version-1".into(),
            offset: 0,
            text: "é".into(),
            eof: true,
        };
        match fault {
            "offset" => value.offset = 1,
            "version" => value.version = "version-2".into(),
            "binary" => value.text = "\0a".into(),
            "empty" => {
                value.text.clear();
                value.eof = false;
            }
            "early-eof" => value.text = "a".into(),
            "missing-eof" => value.eof = false,
            "too-many-bytes" => value.text = "abc".into(),
            "chunk-limit" => value.text = "x".repeat(wire::CHUNK_BYTES + 1),
            _ => {}
        }
        reply(
            &mut host,
            id,
            if fault == "wrong-phase" {
                wire::Response::Stat(metadata(2))
            } else {
                wire::Response::Read(value)
            },
        );
        let result = opened(&mut requester);
        assert!(result.error.is_some(), "{fault}");
        assert!(result.buffer.is_none(), "{fault}");
        assert_eq!(host.app.buffers.len(), count, "{fault}");
        assert_eq!(
            host.app.plugins.instances[&0].application.retained_payload,
            0
        );
        assert!(host.provider_reads.is_empty());
    }
}

#[test]
fn provider_metadata_bounds_and_capabilities_are_enforced_before_reading() {
    for fault in ["label", "key", "version", "syntax", "encoding", "size"] {
        let (_root, mut host) = host();
        let (mut requester, mut provider) = pair(&mut host);
        open(&mut host, &mut requester, 1, None);
        let (id, _) = resource_request(&mut provider);
        let mut value = metadata(0);
        match fault {
            "label" => value.label = "x".repeat(161),
            "key" => value.key = "bad\nkey".into(),
            "version" => value.version = "x".repeat(257),
            "syntax" => value.syntax_hint = Some("bad/hint".into()),
            "encoding" => value.encoding = "utf-16".into(),
            "size" => value.bytes = wire::MAX_DOCUMENT_BYTES + 1,
            _ => unreachable!(),
        }
        reply(&mut host, id, wire::Response::Stat(value));
        assert!(opened(&mut requester).error.is_some(), "{fault}");
        assert!(host.provider_reads.is_empty());
        while let Ok(message) = provider.try_recv() {
            assert!(matches!(message, HostMessage::Deadline { .. }));
        }
    }
    for capabilities in [vec![], vec!["documents"], vec!["jobs"]] {
        let (_root, mut host) = host();
        let mut output = setup(&mut host, 0, &capabilities);
        next(&mut output);
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
            response(&mut output).unwrap_err().code,
            api::ErrorCode::CapabilityDenied
        );
        request(
            &mut host,
            0,
            2,
            api::Request::ProviderRegister(wire::Registration {
                name: "remote".into(),
                conditional_write: false,
                atomic_replace: false,
            }),
        );
        assert_eq!(
            response(&mut output).unwrap_err().code,
            api::ErrorCode::CapabilityDenied
        );
    }
}

#[test]
fn provider_registration_identity_and_inflight_limits_are_bounded() {
    let (_root, mut host) = host();
    let (mut requester, mut provider) = pair(&mut host);
    for (serial, name, code) in [
        (2, "bad/name", api::ErrorCode::InvalidArgument),
        (3, "remote", api::ErrorCode::Conflict),
    ] {
        request(
            &mut host,
            1,
            serial,
            api::Request::ProviderRegister(wire::Registration {
                name: name.into(),
                conditional_write: false,
                atomic_replace: false,
            }),
        );
        assert_eq!(response(&mut provider).unwrap_err().code, code);
    }
    for serial in 4..11 {
        request(
            &mut host,
            1,
            serial,
            api::Request::ProviderRegister(wire::Registration {
                name: format!("provider-{serial}"),
                conditional_write: false,
                atomic_replace: false,
            }),
        );
        response(&mut provider).unwrap();
    }
    request(
        &mut host,
        1,
        11,
        api::Request::ProviderRegister(wire::Registration {
            name: "ninth".into(),
            conditional_write: false,
            atomic_replace: false,
        }),
    );
    assert_eq!(
        response(&mut provider).unwrap_err().code,
        api::ErrorCode::LimitExceeded
    );
    open(&mut host, &mut requester, 1, None);
    open_key(&mut host, &mut requester, 2, "second", None);
    request(
        &mut host,
        0,
        3,
        api::Request::ResourceOpen {
            plugin: "app-1".into(),
            provider: "remote".into(),
            key: "third".into(),
            invocation: None,
        },
    );
    assert_eq!(
        response(&mut requester).unwrap_err().code,
        api::ErrorCode::Busy
    );
    assert_eq!(host.provider_reads.len(), 2);
}

#[test]
fn provider_live_and_concurrent_opens_deduplicate_without_replacing_edits() {
    let (_root, mut host) = host();
    let (mut requester, mut provider) = pair(&mut host);
    open(&mut host, &mut requester, 1, None);
    request(
        &mut host,
        0,
        2,
        api::Request::ResourceOpen {
            plugin: "app-1".into(),
            provider: "remote".into(),
            key: "requested/猫.txt".into(),
            invocation: None,
        },
    );
    assert_eq!(
        response(&mut requester).unwrap_err().code,
        api::ErrorCode::Busy
    );
    let (first, _) = stat(&mut host, &mut provider, 2);
    open_key(&mut host, &mut requester, 3, "another-alias", None);
    let (alias, _) = resource_request(&mut provider);
    reply(&mut host, alias, wire::Response::Stat(metadata(2)));
    assert_eq!(
        opened(&mut requester).error.unwrap().code,
        api::ErrorCode::Busy
    );
    chunk(&mut host, first, 0, "é", true);
    let first = opened(&mut requester).buffer.unwrap();
    let buffer = host.app.plugins.instances[&0].application.buffers[&first];
    edit(&mut host, buffer, "local ");
    assert_eq!(host.app.buffers[buffer].to_string(), "local é");
    assert!(host.app.buffers[buffer].dirty);
    open(&mut host, &mut requester, 4, None);
    let (stat_id, _) = resource_request(&mut provider);
    let mut newer = metadata(7);
    newer.version = "version-2".into();
    reply(&mut host, stat_id, wire::Response::Stat(newer));
    assert_eq!(opened(&mut requester).buffer.as_ref(), Some(&first));
    assert_eq!(host.app.buffers[buffer].to_string(), "local é");
    assert_eq!(
        host.app.buffers[buffer].provider().unwrap().version,
        "version-1"
    );
    assert_eq!(
        host.app
            .buffers
            .iter()
            .filter(|buffer| buffer.provider().is_some())
            .count(),
        1
    );
}

#[test]
fn provider_cancellation_deadlines_and_late_replies_settle_once_and_release_payload() {
    for cancel in ["cancel", "request-deadline", "job-deadline"] {
        let (_root, mut host) = host();
        let (mut requester, mut provider) = pair(&mut host);
        let token = open(&mut host, &mut requester, 1, None);
        let (id, _) = stat(&mut host, &mut provider, 2);
        match cancel {
            "cancel" => {
                request(
                    &mut host,
                    0,
                    2,
                    api::Request::JobCancel { job: token.clone() },
                );
            }
            _ => {
                let (owner, deadline) = if cancel == "request-deadline" {
                    (1, id.clone())
                } else {
                    (0, token.clone())
                };
                *host
                    .app
                    .plugins
                    .instances
                    .get_mut(&owner)
                    .unwrap()
                    .application
                    .deadlines
                    .get_mut(&deadline)
                    .unwrap() = std::time::Instant::now();
                host.handle_plugin_event(Event {
                    plugin: owner,
                    result: Ok(ClientMessage::Deadline { token: deadline }),
                });
            }
        }
        let result = opened(&mut requester);
        assert_eq!(
            result.error.unwrap().code,
            if cancel == "cancel" {
                api::ErrorCode::Cancelled
            } else {
                api::ErrorCode::Timeout
            }
        );
        if cancel == "cancel" {
            assert!(
                matches!(response(&mut requester).unwrap(), api::ResultValue::Job(job) if job.state == api::JobState::Cancelled)
            );
        }
        assert!(host.provider_reads.is_empty());
        assert_eq!(
            host.app.plugins.instances[&0].application.retained_payload,
            0
        );
        let count = host.app.buffers.len();
        chunk(&mut host, id, 0, "é", true);
        assert_eq!(host.app.buffers.len(), count);
        assert!(requester.try_recv().is_err());
        assert!(!host.provider_deadline(0, &token));
    }
}

#[test]
fn provider_stop_fails_pending_reads_and_marks_open_documents_unavailable_without_losing_edits() {
    let (_root, mut host) = host();
    let (mut requester, mut provider) = pair(&mut host);
    open(&mut host, &mut requester, 1, None);
    let (id, _) = stat(&mut host, &mut provider, 2);
    chunk(&mut host, id, 0, "é", true);
    let handle = opened(&mut requester).buffer.unwrap();
    let buffer = host.app.plugins.instances[&0].application.buffers[&handle];
    edit(&mut host, buffer, "retained ");
    open(&mut host, &mut requester, 2, None);
    resource_request(&mut provider);
    host.stop_plugin(1, "provider stopped in test");
    assert_eq!(
        opened(&mut requester).error.unwrap().code,
        api::ErrorCode::Unavailable
    );
    assert!(!host.app.buffers[buffer].provider().unwrap().available);
    assert_eq!(host.app.buffers[buffer].to_string(), "retained é");
    assert!(host.app.buffers[buffer].dirty);
    assert_eq!(
        host.app.plugins.instances[&0].application.retained_payload,
        0
    );
    request(
        &mut host,
        0,
        3,
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
}

#[test]
fn provider_foreground_publication_respects_newer_input_and_requester_stop() {
    for outcome in ["show", "input", "target", "detach", "stop"] {
        let (_root, mut host) = host();
        let (mut requester, mut provider) = pair(&mut host);
        host.app.note_plugin_frontend(true);
        invoke(&mut host, "plugin.app-0.open");
        let api::HostMessage::Request { id: invocation, .. } = next(&mut requester) else {
            panic!()
        };
        open(&mut host, &mut requester, 1, Some(invocation));
        let (id, _) = stat(&mut host, &mut provider, 2);
        if outcome == "stop" {
            host.stop_plugin(0, "requester stopped in test");
            assert!(host.provider_reads.is_empty());
            let count = host.app.buffers.len();
            chunk(&mut host, id, 0, "é", true);
            assert_eq!(host.app.buffers.len(), count);
            assert!(host.app.plugins.instances.contains_key(&1));
        } else {
            match outcome {
                "input" => host.app.handle_key(KeyStroke::char('l')).unwrap(),
                "target" => {
                    invoke(&mut host, "new");
                }
                "detach" => host.app.note_plugin_frontend(false),
                _ => {}
            }
            let active = host.app.active().buffer;
            chunk(&mut host, id, 0, "é", true);
            let result = opened(&mut requester);
            let handle = result.buffer.unwrap();
            let buffer = host.app.plugins.instances[&0].application.buffers[&handle];
            if outcome == "show" {
                assert_eq!(host.app.active().buffer, buffer);
            } else {
                assert_eq!(host.app.active().buffer, active, "{outcome}");
            }
        }
    }
}

#[test]
fn provider_stat_phase_rejects_wrong_responses_and_propagates_structured_failures() {
    for failure in [false, true] {
        let (_root, mut host) = host();
        let (mut requester, mut provider) = pair(&mut host);
        open(&mut host, &mut requester, 1, None);
        let (id, _) = resource_request(&mut provider);
        let outcome = if failure {
            api::CommandResponse::Failure {
                error: api::Error::new(api::ErrorCode::Unavailable, "Resource is offline"),
            }
        } else {
            api::CommandResponse::Success {
                result: api::CommandResult { job: None },
            }
        };
        host.application_message(1, api::ClientMessage::Response { id, outcome })
            .unwrap();
        let result = opened(&mut requester);
        assert_eq!(
            result.error.unwrap().code,
            if failure {
                api::ErrorCode::Unavailable
            } else {
                api::ErrorCode::InvalidArgument
            }
        );
        assert!(result.buffer.is_none());
        assert!(host.provider_reads.is_empty());
        assert_eq!(
            host.app.plugins.instances[&0].application.retained_payload,
            0
        );
    }
}
