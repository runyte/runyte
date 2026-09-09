// SPDX-License-Identifier: MPL-2.0

use super::filesystem::response;
use super::providers::{chunk, open, opened, pair, reply, resource_request, stat};
use super::*;
use crate::plugin::provider as wire;

pub(super) fn document(
    host: &mut WorkspaceHost,
    text: &str,
) -> (
    mpsc::Receiver<HostMessage>,
    mpsc::Receiver<HostMessage>,
    String,
    usize,
) {
    let (mut requester, mut provider) = pair(host);
    open(host, &mut requester, 1, None);
    let (id, _) = stat(host, &mut provider, text.len());
    chunk(host, id, 0, text, true);
    let handle = opened(&mut requester).buffer.unwrap();
    let index = host.app.plugins.instances[&0].application.buffers[&handle];
    while let Ok(message) = provider.try_recv() {
        assert!(matches!(
            message,
            HostMessage::Deadline { .. }
                | HostMessage::Application(api::HostMessage::Event {
                    event: "resource.released",
                    ..
                })
        ));
    }
    (requester, provider, handle, index)
}

pub(super) fn save(
    host: &mut WorkspaceHost,
    requester: &mut mpsc::Receiver<HostMessage>,
    handle: &str,
    index: usize,
) -> api::Job {
    request(
        host,
        0,
        10,
        api::Request::BufferSave {
            buffer: handle.into(),
            expected_revision: format!("r:{}", host.app.buffers[index].revision()),
        },
    );
    let result = job(requester);
    assert!(matches!(
        next(requester),
        api::HostMessage::Event {
            event: "job.changed",
            ..
        }
    ));
    result
}

fn applications(output: &mut mpsc::Receiver<HostMessage>) -> Vec<api::HostMessage> {
    let mut messages = Vec::new();
    while let Ok(message) = output.try_recv() {
        match message {
            HostMessage::Application(message) => messages.push(message),
            HostMessage::Deadline { .. } => {}
            other => panic!("unexpected {other:?}"),
        }
    }
    messages
}

pub(super) fn upload(
    host: &mut WorkspaceHost,
    provider: &mut mpsc::Receiver<HostMessage>,
    begin: (String, wire::Request),
) -> (String, String) {
    let (id, begin) = begin;
    let wire::Request::WriteBegin {
        mode,
        bytes,
        expected_version,
        encoding,
        provider: name,
        key,
        ..
    } = begin
    else {
        panic!("{begin:?}")
    };
    assert_eq!(expected_version, "version-1");
    assert_eq!(encoding, "utf-8");
    assert_eq!(name, "remote");
    assert_eq!(key, "canonical/猫.txt");
    reply(
        host,
        id,
        wire::Response::WriteStarted {
            upload: "upload-1".into(),
        },
    );
    let mut captured = String::new();
    loop {
        let (id, request) = resource_request(provider);
        match request {
            wire::Request::WriteChunk {
                upload,
                offset,
                text,
                ..
            } => {
                assert_eq!(upload, "upload-1");
                assert_eq!(offset, captured.len());
                assert!(!text.is_empty());
                assert!(text.len() <= wire::CHUNK_BYTES);
                captured.push_str(&text);
                reply(
                    host,
                    id,
                    wire::Response::WriteChunk {
                        offset: captured.len(),
                    },
                );
            }
            wire::Request::WriteCommit {
                mode: commit_mode,
                upload,
                expected_version,
                ..
            } => {
                assert_eq!(mode, commit_mode);
                assert_eq!(upload, "upload-1");
                assert_eq!(expected_version, "version-1");
                assert_eq!(captured.len(), bytes);
                return (id, captured);
            }
            other => panic!("unexpected {other:?}"),
        }
    }
}

pub(super) fn finished(
    output: &mut mpsc::Receiver<HostMessage>,
    expected: api::JobState,
) -> wire::Finished {
    let mut result = None;
    let mut state = None;
    for _ in 0..2 {
        match next(output) {
            api::HostMessage::Event {
                event: "resource.saved",
                data: api::EventData::ResourceFinished(value),
                ..
            } => result = Some(value),
            api::HostMessage::Event {
                event: "job.changed",
                data: api::EventData::Job(job),
                ..
            } => state = Some(job.state),
            other => panic!("unexpected completion {other:?}"),
        }
    }
    assert_eq!(state, Some(expected));
    result.unwrap()
}

#[test]
fn provider_write_captures_unicode_chunks_and_accepts_only_uploaded_baseline() {
    let (root, mut host) = host();
    let before = std::fs::read_dir(root.path())
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect::<std::collections::BTreeSet<_>>();
    let (mut requester, mut provider, handle, index) = document(&mut host, "base\n");
    let authored = format!("{}猫é\n", "é猫".repeat(wire::CHUNK_BYTES / 5));
    host.app.buffers[index].apply(&Transaction::insert(0, &authored));
    let snapshot = host.app.buffers[index].to_string();
    let job = save(&mut host, &mut requester, &handle, index);
    let saved_revision = format!("r:{}", host.app.buffers[index].revision());
    assert!(host.app.document_mutation_pending(index));
    let revision = format!("r:{}", host.app.buffers[index].revision());
    request(
        &mut host,
        0,
        11,
        api::Request::BufferSave {
            buffer: handle.clone(),
            expected_revision: revision,
        },
    );
    assert_eq!(
        response(&mut requester).unwrap_err().code,
        api::ErrorCode::Busy
    );
    let begin = resource_request(&mut provider);
    assert!(
        host.application_message(
            0,
            api::ClientMessage::Response {
                id: begin.0.clone(),
                outcome: api::CommandResponse::Resource {
                    result: wire::Response::WriteStarted {
                        upload: "forged".into()
                    }
                }
            }
        )
        .is_err()
    );
    host.app.buffers[index].apply(&Transaction::insert(0, "newer "));
    host.app.buffers[index].commit_undo_group();
    let (id, uploaded) = upload(&mut host, &mut provider, begin);
    assert_eq!(uploaded, snapshot);
    reply(
        &mut host,
        id.clone(),
        wire::Response::WriteCommitted {
            version: "version-2".into(),
        },
    );
    let result = finished(&mut requester, api::JobState::Succeeded);
    assert_eq!(result.job, job.job);
    assert_eq!(result.buffer.as_deref(), Some(handle.as_str()));
    assert_eq!(result.revision.as_deref(), Some(saved_revision.as_str()));
    assert_ne!(
        saved_revision,
        format!("r:{}", host.app.buffers[index].revision())
    );
    assert!(result.error.is_none());
    assert!(host.app.buffers[index].dirty);
    assert_eq!(
        host.app.buffers[index].provider().unwrap().version,
        "version-2"
    );
    assert!(host.app.buffers[index].undo());
    assert_eq!(host.app.buffers[index].to_string(), snapshot);
    assert!(!host.app.buffers[index].dirty);
    assert!(!host.app.document_mutation_pending(index));
    assert!(host.provider_writes.is_empty());
    assert_eq!(
        host.app.plugins.instances[&0].application.retained_payload,
        0
    );
    assert_eq!(
        host.app.plugins.instances[&1].application.provider_requests,
        0
    );
    assert!(
        host.application_message(
            1,
            api::ClientMessage::Response {
                id,
                outcome: api::CommandResponse::Resource {
                    result: wire::Response::WriteCommitted {
                        version: "duplicate".into()
                    }
                }
            }
        )
        .is_err()
    );
    let after = std::fs::read_dir(root.path())
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(before, after);
}

#[test]
fn native_provider_save_rejects_edits_before_host_admission_without_trimming_or_upload() {
    for command in ["write", "wq", "wbc"] {
        let (_root, mut host) = host();
        let (mut requester, mut provider, _, index) = document(&mut host, "base  \n");
        host.app.config.editor.trim_trailing_whitespace = true;
        host.app.show_provider_document(index);
        let queued_revision = host.app.buffers[index].revision();
        type_command(&mut host, command);
        assert_eq!(
            host.app.plugins.provider_save_intents[0].expected_revision,
            queued_revision
        );
        assert!(host.app.document_mutation_pending(index));
        host.app.buffers[index].apply(&Transaction::insert(0, "newer  \n"));
        let live_revision = host.app.buffers[index].revision();
        assert_ne!(queued_revision, live_revision);
        host.sync_plugin_observers();
        assert!(host.app.plugins.provider_save_intents.is_empty());
        assert!(host.provider_writes.is_empty());
        assert!(!host.app.document_mutation_pending(index));
        assert_eq!(host.app.buffers[index].to_string(), "newer  \nbase  \n");
        assert_eq!(host.app.buffers[index].revision(), live_revision);
        assert!(host.app.buffers[index].dirty);
        assert!(!host.app.host_buffer_is_closed(index));
        assert!(!host.app.should_quit);
        assert!(host.app.status_error);
        assert!(applications(&mut requester).is_empty());
        assert!(applications(&mut provider).is_empty());
        assert!(host.app.plugins.instances[&1].application.jobs.is_empty());
        assert_eq!(
            host.app.plugins.instances[&1].application.retained_payload,
            0
        );
        assert_eq!(
            host.app.plugins.instances[&1].application.provider_requests,
            0
        );
    }
}

#[test]
fn native_provider_save_close_waits_for_real_empty_commit_with_provider_only_capability() {
    let (_root, mut host) = host();
    let (_requester, mut provider, _, index) = document(&mut host, "");
    host.app.show_provider_document(index);
    type_command(&mut host, "wbc");
    assert!(!host.app.host_buffer_is_closed(index));
    assert!(host.protected_state().plugin_jobs > 0);
    host.sync_plugin_observers();
    let begin = resource_request(&mut provider);
    assert!(matches!(
        next(&mut provider),
        api::HostMessage::Event {
            event: "job.changed",
            ..
        }
    ));
    assert!(!host.app.host_buffer_is_closed(index));
    let (id, text) = upload(&mut host, &mut provider, begin);
    assert!(text.is_empty());
    assert!(!host.app.host_buffer_is_closed(index));
    reply(
        &mut host,
        id,
        wire::Response::WriteCommitted {
            version: "version-2".into(),
        },
    );
    finished(&mut provider, api::JobState::Succeeded);
    assert!(host.app.host_buffer_is_closed(index));
    assert!(!host.app.should_quit);
    assert!(host.provider_writes.is_empty());
}

#[test]
fn provider_commit_rejection_preserves_conflict_and_latches_explicit_unknown_outcome() {
    for code in [api::ErrorCode::Conflict, api::ErrorCode::OutcomeUnknown] {
        let (_root, mut host) = host();
        let (mut requester, mut provider, handle, index) = document(&mut host, "base");
        host.app.buffers[index].apply(&Transaction::insert(0, "local "));
        save(&mut host, &mut requester, &handle, index);
        let begin = resource_request(&mut provider);
        let (id, _) = upload(&mut host, &mut provider, begin);
        reply(
            &mut host,
            id,
            wire::Response::WriteRejected {
                error: api::Error::new(code.clone(), "Remote conflict or uncertainty"),
            },
        );
        let unknown = code == api::ErrorCode::OutcomeUnknown;
        let result = finished(
            &mut requester,
            if unknown {
                api::JobState::OutcomeUnknown
            } else {
                api::JobState::Failed
            },
        );
        assert!(result.revision.is_none());
        assert_eq!(result.error.unwrap().code, code);
        assert_eq!(host.app.buffers[index].to_string(), "local base");
        assert!(host.app.buffers[index].dirty);
        assert_eq!(
            host.app.buffers[index].provider().unwrap().version,
            "version-1"
        );
        assert_eq!(
            host.app.buffers[index]
                .provider()
                .unwrap()
                .uncertain
                .is_some(),
            unknown
        );
        assert_eq!(host.provider_uncertain.contains_key(&index), unknown);
        assert_eq!(
            host.app.plugins.orphaned_payload,
            if unknown { wire::READ_CHARGE } else { 0 }
        );
    }
}

#[test]
fn provider_write_cancellation_keeps_guard_until_abort_and_ignores_late_begin() {
    let (_root, mut host) = host();
    let (mut requester, mut provider, handle, index) = document(&mut host, "base");
    let job = save(&mut host, &mut requester, &handle, index);
    let (begin, _) = resource_request(&mut provider);
    request(
        &mut host,
        0,
        11,
        api::Request::JobCancel {
            job: job.job.clone(),
        },
    );
    let api::ResultValue::Job(cancelled) = response(&mut requester).unwrap() else {
        panic!()
    };
    assert_eq!(cancelled.state, api::JobState::Cancelling);
    next(&mut requester);
    let (abort, request) = resource_request(&mut provider);
    assert!(matches!(
        request,
        wire::Request::WriteAbort { upload: None, .. }
    ));
    assert!(host.app.document_mutation_pending(index));
    assert!(host.app.host_close_buffer(index, true).is_err());
    reply(
        &mut host,
        begin,
        wire::Response::WriteStarted {
            upload: "late-staging".into(),
        },
    );
    assert!(applications(&mut provider).is_empty());
    reply(&mut host, abort, wire::Response::WriteAborted {});
    finished(&mut requester, api::JobState::Cancelled);
    assert!(!host.app.document_mutation_pending(index));
    assert!(
        host.app.buffers[index]
            .provider()
            .unwrap()
            .uncertain
            .is_none()
    );
    assert_eq!(
        host.app.plugins.instances[&0].application.retained_payload,
        0
    );
}

#[test]
fn provider_post_commit_cancel_and_late_success_cannot_clear_uncertainty() {
    let (_root, mut host) = host();
    let (mut requester, mut provider, handle, index) = document(&mut host, "base");
    let job = save(&mut host, &mut requester, &handle, index);
    let begin = resource_request(&mut provider);
    let (commit, _) = upload(&mut host, &mut provider, begin);
    request(
        &mut host,
        0,
        11,
        api::Request::JobCancel {
            job: job.job.clone(),
        },
    );
    let messages = applications(&mut requester);
    assert_eq!(messages.iter().filter(|message| matches!(message, api::HostMessage::Event { event: "job.changed", data: api::EventData::Job(job), .. } if !job.state.active())).count(), 1);
    assert!(messages.iter().any(|message| matches!(message, api::HostMessage::Event { event: "resource.saved", data: api::EventData::ResourceFinished(result), .. } if result.error.as_ref().is_some_and(|error| error.code == api::ErrorCode::OutcomeUnknown))));
    assert_eq!(
        host.app.plugins.instances[&0].application.jobs[&job.job].state,
        api::JobState::OutcomeUnknown
    );
    assert!(host.app.buffers[index].dirty);
    reply(
        &mut host,
        commit,
        wire::Response::WriteCommitted {
            version: "late-version".into(),
        },
    );
    assert!(host.app.buffers[index].dirty);
    assert_eq!(
        host.app.buffers[index].provider().unwrap().version,
        "version-1"
    );
    assert!(host.app.buffers[index].prepare_provider_save().is_err());
    assert!(applications(&mut provider).is_empty());
}

#[test]
fn stopped_provider_write_requester_remains_protected_until_provider_aborts() {
    let (_root, mut host) = host();
    let (mut requester, mut provider, handle, index) = document(&mut host, "base");
    save(&mut host, &mut requester, &handle, index);
    resource_request(&mut provider);
    host.stop_plugin(0, "test requester stop");
    assert!(!host.app.plugins.instances.contains_key(&0));
    assert!(host.app.plugins.instances.contains_key(&1));
    assert!(host.app.document_mutation_pending(index));
    assert_eq!(host.app.plugins.orphaned_payload, wire::READ_CHARGE);
    assert_eq!(host.protected_state().plugin_jobs, 1);
    let (abort, request) = resource_request(&mut provider);
    assert!(matches!(request, wire::Request::WriteAbort { .. }));
    reply(&mut host, abort, wire::Response::WriteAborted {});
    assert!(host.provider_writes.is_empty());
    assert_eq!(host.app.plugins.orphaned_payload, 0);
    assert!(!host.app.document_mutation_pending(index));
    assert!(!host.app.buffers[index].dirty);
}

#[test]
fn provider_abort_deadline_does_not_stop_the_separate_requester() {
    let (_root, mut host) = host();
    let (mut requester, mut provider, handle, index) = document(&mut host, "base");
    let job = save(&mut host, &mut requester, &handle, index);
    resource_request(&mut provider);
    request(
        &mut host,
        0,
        11,
        api::Request::JobCancel {
            job: job.job.clone(),
        },
    );
    applications(&mut requester);
    let (abort, _) = resource_request(&mut provider);
    host.app
        .plugins
        .instances
        .get_mut(&0)
        .unwrap()
        .application
        .deadlines
        .insert(job.job.clone(), std::time::Instant::now());
    host.handle_plugin_event(Event {
        plugin: 0,
        result: Ok(ClientMessage::Deadline { token: job.job }),
    });
    assert!(host.app.plugins.instances.contains_key(&0));
    assert!(host.app.plugins.instances.contains_key(&1));
    assert!(host.app.document_mutation_pending(index));
    host.app
        .plugins
        .instances
        .get_mut(&1)
        .unwrap()
        .application
        .deadlines
        .insert(abort.clone(), std::time::Instant::now());
    host.handle_plugin_event(Event {
        plugin: 1,
        result: Ok(ClientMessage::Deadline { token: abort }),
    });
    assert!(host.app.plugins.instances.contains_key(&0));
    assert!(!host.app.plugins.instances.contains_key(&1));
    assert!(!host.app.document_mutation_pending(index));
}

#[test]
fn provider_write_full_queue_at_begin_or_commit_releases_job_and_accounts_uncertainty() {
    for committed in [false, true] {
        let (_root, mut host) = host();
        let (mut requester, mut provider, handle, index) = document(&mut host, "base");
        let commit = if committed {
            save(&mut host, &mut requester, &handle, index);
            let begin = resource_request(&mut provider);
            Some(upload(&mut host, &mut provider, begin).0)
        } else {
            None
        };
        while host.app.plugins.instances[&1]
            .sender
            .try_send(HostMessage::Deadline {
                token: "filler".into(),
                after_ms: None,
            })
            .is_ok()
        {}
        if let Some(id) = commit {
            host.handle_plugin_event(Event {
                plugin: 1,
                result: Ok(ClientMessage::Application(api::ClientMessage::Response {
                    id,
                    outcome: api::CommandResponse::Resource {
                        result: wire::Response::WriteCommitted {
                            version: "version-2".into(),
                        },
                    },
                })),
            });
            finished(&mut requester, api::JobState::OutcomeUnknown);
        } else {
            let revision = format!("r:{}", host.app.buffers[index].revision());
            request(
                &mut host,
                0,
                10,
                api::Request::BufferSave {
                    buffer: handle,
                    expected_revision: revision,
                },
            );
            assert_eq!(
                response(&mut requester).unwrap_err().code,
                api::ErrorCode::Unavailable
            );
        }
        assert!(!host.app.plugins.instances.contains_key(&1));
        assert!(host.app.plugins.instances.contains_key(&0));
        assert!(host.provider_writes.is_empty());
        assert!(!host.app.document_mutation_pending(index));
        assert_eq!(host.app.buffers[index].dirty, committed);
        assert_eq!(
            host.app.buffers[index]
                .provider()
                .unwrap()
                .uncertain
                .is_some(),
            committed
        );
        assert_eq!(
            host.app.plugins.instances[&0].application.retained_payload,
            0
        );
    }
}
