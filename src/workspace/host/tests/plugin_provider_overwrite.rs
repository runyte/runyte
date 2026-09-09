// SPDX-License-Identifier: MPL-2.0

use super::filesystem::response;
use super::provider_writes::{document, finished, upload};
use super::providers::{reply, resource_request};
use super::*;
use crate::{
    input::{KeyCode, KeyStroke},
    plugin::provider as wire,
};

fn weak(
    host: &mut WorkspaceHost,
    provider: &mut mpsc::Receiver<HostMessage>,
    index: usize,
    command: &str,
) -> String {
    host.app
        .plugins
        .instances
        .get_mut(&1)
        .unwrap()
        .application
        .providers
        .get_mut("remote")
        .unwrap()
        .conditional_write = false;
    host.app.show_provider_document(index);
    host.app.note_plugin_frontend(true);
    invoke(host, command);
    host.sync_provider_writes();
    let api::HostMessage::Event {
        event: "job.changed",
        data: api::EventData::Job(job),
        ..
    } = next(provider)
    else {
        panic!()
    };
    assert!(host.app.has_input_overlay());
    assert!(host.app.document_mutation_pending(index));
    assert_eq!(
        host.app.plugins.instances[&1].application.provider_requests,
        0
    );
    assert_eq!(host.provider_writes.len(), 1);
    assert_no_resource(provider);
    job.job
}

fn assert_no_resource(output: &mut mpsc::Receiver<HostMessage>) {
    while let Ok(message) = output.try_recv() {
        assert!(
            matches!(message, HostMessage::Deadline { .. }),
            "{message:?}"
        );
    }
}

fn enter(host: &mut WorkspaceHost) {
    host.app
        .handle_key(KeyStroke::plain(KeyCode::Enter))
        .unwrap();
    host.sync_provider_writes();
}

#[test]
fn provider_overwrite_cancel_preserves_untrimmed_text_and_releases_job_without_provider_calls() {
    for outcome in [
        "escape",
        "cancel",
        "timeout",
        "detach",
        "edit",
        "baseline",
        "capability",
        "stop",
    ] {
        let (_root, mut host) = host();
        let (_requester, mut provider, _handle, index) = document(&mut host, "base  \r\n");
        host.app.config.editor.trim_trailing_whitespace = true;
        host.app.buffers[index].apply(&Transaction::insert(0, "local "));
        let original = host.app.buffers[index].to_string();
        let revision = host.app.buffers[index].revision();
        let job = weak(&mut host, &mut provider, index, "write");
        assert_eq!(host.app.buffers[index].revision(), revision);
        match outcome {
            "escape" => host
                .app
                .handle_key(KeyStroke::plain(KeyCode::Escape))
                .unwrap(),
            "cancel" => {
                host.provider_write_job_request(1, &api::Request::JobCancel { job: job.clone() })
                    .unwrap()
                    .unwrap();
            }
            "timeout" => {
                assert!(host.provider_write_deadline(1, &job).unwrap());
            }
            "detach" => host.app.note_plugin_frontend(false),
            "edit" => {
                host.app.buffers[index].apply(&Transaction::insert(0, "new "));
            }
            "baseline" => {
                host.app.buffers[index]
                    .provider_mut()
                    .unwrap()
                    .baseline_epoch += 1
            }
            "capability" => {
                host.app
                    .plugins
                    .instances
                    .get_mut(&1)
                    .unwrap()
                    .application
                    .providers
                    .get_mut("remote")
                    .unwrap()
                    .atomic_replace = false
            }
            _ => host.stop_plugin(1, "test pending overwrite stop"),
        }
        host.sync_provider_writes();
        assert!(host.provider_writes.is_empty(), "{outcome}");
        assert!(!host.app.has_input_overlay(), "{outcome}");
        assert!(!host.app.document_mutation_pending(index));
        assert_eq!(
            host.app.buffers[index].to_string(),
            if outcome == "edit" {
                format!("new {original}")
            } else {
                original
            }
        );
        assert!(host.app.buffers[index].dirty);
        assert_eq!(host.app.plugins.orphaned_payload, 0);
        if outcome != "stop" {
            let expected = if matches!(outcome, "timeout" | "baseline" | "capability") {
                api::JobState::Failed
            } else {
                api::JobState::Cancelled
            };
            assert!(finished(&mut provider, expected).error.is_some());
            assert_eq!(
                host.app.plugins.instances[&1].application.retained_payload,
                0
            );
            assert_eq!(
                host.app.plugins.instances[&1].application.provider_requests,
                0
            );
            assert_no_resource(&mut provider);
        }
    }
}

#[test]
fn provider_overwrite_approval_uploads_captured_hooks_and_only_then_advances_baseline() {
    let (_root, mut host) = host();
    let (_requester, mut provider, _handle, index) = document(&mut host, "猫  \r\n");
    host.app.config.editor.trim_trailing_whitespace = true;
    host.app.buffers[index].apply(&Transaction::insert(0, "local "));
    weak(&mut host, &mut provider, index, "write");
    host.app.config.editor.trim_trailing_whitespace = false;
    enter(&mut host);
    assert!(!host.app.has_input_overlay());
    assert_eq!(host.app.buffers[index].to_string(), "local 猫\r\n");
    let begin = resource_request(&mut provider);
    assert!(matches!(
        begin.1,
        wire::Request::WriteBegin {
            mode: wire::WriteMode::ConfirmedBestEffort,
            ..
        }
    ));
    let (commit, uploaded) = upload(&mut host, &mut provider, begin);
    assert_eq!(uploaded, "local 猫\r\n");
    let captured = host.app.buffers[index].revision();
    host.app.buffers[index].apply(&Transaction::insert(0, "newer "));
    reply(
        &mut host,
        commit,
        wire::Response::WriteCommitted {
            version: "saved".into(),
        },
    );
    let result = finished(&mut provider, api::JobState::Succeeded);
    assert_eq!(result.revision, Some(format!("r:{captured}")));
    assert_eq!(host.app.buffers[index].to_string(), "newer local 猫\r\n");
    assert!(host.app.buffers[index].dirty);
    assert_eq!(host.app.buffers[index].provider().unwrap().version, "saved");
    assert_eq!(
        host.app.plugins.instances[&1].application.retained_payload,
        0
    );
}

#[test]
fn provider_overwrite_busy_approval_stays_visible_and_requires_another_enter() {
    let (_root, mut host) = host();
    let (_requester, mut provider, _handle, index) = document(&mut host, "base  \n");
    host.app.config.editor.trim_trailing_whitespace = true;
    weak(&mut host, &mut provider, index, "write");
    host.app
        .plugins
        .instances
        .get_mut(&1)
        .unwrap()
        .application
        .provider_requests = api::MAX_REQUESTS;
    enter(&mut host);
    assert!(host.app.has_input_overlay());
    assert_eq!(host.app.buffers[index].to_string(), "base  \n");
    assert_no_resource(&mut provider);
    host.app
        .plugins
        .instances
        .get_mut(&1)
        .unwrap()
        .application
        .provider_requests = 0;
    host.sync_provider_writes();
    assert!(host.app.has_input_overlay());
    assert_no_resource(&mut provider);
    enter(&mut host);
    assert!(matches!(
        resource_request(&mut provider).1,
        wire::Request::WriteBegin {
            mode: wire::WriteMode::ConfirmedBestEffort,
            ..
        }
    ));
    assert_eq!(host.app.buffers[index].to_string(), "base\n");
}

#[test]
fn provider_overwrite_stale_accepted_context_never_applies_hooks_or_starts_upload() {
    let (_root, mut host) = host();
    let (_requester, mut provider, _handle, index) = document(&mut host, "base  \n");
    host.app.config.editor.trim_trailing_whitespace = true;
    weak(&mut host, &mut provider, index, "write");
    host.app
        .handle_key(KeyStroke::plain(KeyCode::Enter))
        .unwrap();
    host.app.handle_key(KeyStroke::char('l')).unwrap();
    host.sync_provider_writes();
    assert!(
        finished(&mut provider, api::JobState::Failed)
            .error
            .is_some()
    );
    assert_eq!(host.app.buffers[index].to_string(), "base  \n");
    assert_no_resource(&mut provider);
    assert!(!host.app.document_mutation_pending(index));
}

#[test]
fn provider_overwrite_save_and_close_refreshes_the_confirmation_context() {
    for command in ["write-buffer-close", "write-quit"] {
        let (_root, mut host) = host();
        let (_requester, mut provider, _handle, index) = document(&mut host, "base");
        host.app.buffers[index].apply(&Transaction::insert(0, "local "));
        weak(&mut host, &mut provider, index, command);
        enter(&mut host);
        let begin = resource_request(&mut provider);
        let (commit, _) = upload(&mut host, &mut provider, begin);
        reply(
            &mut host,
            commit,
            wire::Response::WriteCommitted {
                version: "saved".into(),
            },
        );
        assert!(
            finished(&mut provider, api::JobState::Succeeded)
                .error
                .is_none()
        );
        if command == "write-buffer-close" {
            assert!(host.app.host_buffer_is_closed(index));
        } else {
            assert!(host.app.should_quit);
        }
    }
}

#[test]
fn provider_overwrite_public_save_cannot_offer_or_forge_confirmation() {
    let (_root, mut host) = host();
    let (mut requester, mut provider, handle, index) = document(&mut host, "base  \n");
    host.app
        .plugins
        .instances
        .get_mut(&1)
        .unwrap()
        .application
        .providers
        .get_mut("remote")
        .unwrap()
        .conditional_write = false;
    host.app.show_provider_document(index);
    host.app.note_plugin_frontend(true);
    let expected_revision = format!("r:{}", host.app.buffers[index].revision());
    request(
        &mut host,
        0,
        2,
        api::Request::BufferSave {
            buffer: handle,
            expected_revision,
        },
    );
    assert_eq!(
        response(&mut requester).unwrap_err().code,
        api::ErrorCode::Unsupported
    );
    assert!(!host.app.has_input_overlay());
    assert!(host.provider_writes.is_empty());
    assert_eq!(host.app.buffers[index].to_string(), "base  \n");
    assert_no_resource(&mut provider);
}

#[test]
fn provider_overwrite_reserves_completion_handle_and_unknown_outcome_keeps_only_recovery_charge() {
    let (_root, mut host) = host();
    let (_requester, mut provider, _handle, index) = document(&mut host, "base");
    host.app.buffers[index].apply(&Transaction::insert(0, "local "));
    let job = weak(&mut host, &mut provider, index, "write");
    let state = &mut host.app.plugins.instances.get_mut(&1).unwrap().application;
    let handle = state
        .buffers
        .iter()
        .find(|(_, target)| **target == index)
        .unwrap()
        .0
        .clone();
    assert!(state.retained_payload > wire::READ_CHARGE);
    while state.buffers.len() < 1024 {
        state
            .buffers
            .insert(format!("filler:{}", state.buffers.len()), 0);
    }
    enter(&mut host);
    let begin = resource_request(&mut provider);
    let (commit, _) = upload(&mut host, &mut provider, begin);
    assert!(host.provider_write_deadline(1, &commit).unwrap());
    let result = finished(&mut provider, api::JobState::OutcomeUnknown);
    assert_eq!(result.buffer, Some(handle));
    assert_eq!(result.error.unwrap().code, api::ErrorCode::OutcomeUnknown);
    assert_eq!(
        host.provider_uncertain[&index],
        ("app-1".into(), wire::READ_CHARGE, job)
    );
    assert_eq!(
        host.app.plugins.instances[&1].application.retained_payload,
        0
    );
    assert_eq!(host.app.plugins.orphaned_payload, wire::READ_CHARGE);
    assert!(host.app.buffers[index].dirty);
    reply(
        &mut host,
        commit,
        wire::Response::WriteCommitted {
            version: "late".into(),
        },
    );
    assert_eq!(
        host.app.buffers[index].provider().unwrap().version,
        "version-1"
    );
    assert_no_resource(&mut provider);
}

#[test]
fn provider_overwrite_preview_limit_is_actionable_and_leaves_no_job_or_handle() {
    let (_root, mut host) = host();
    let (mut requester, mut provider, _handle, index) = document(&mut host, &"x  \n".repeat(4097));
    host.app.config.editor.trim_trailing_whitespace = true;
    host.app
        .plugins
        .instances
        .get_mut(&1)
        .unwrap()
        .application
        .providers
        .get_mut("remote")
        .unwrap()
        .conditional_write = false;
    host.app.show_provider_document(index);
    host.app.note_plugin_frontend(true);
    invoke(&mut host, "plugin.app-0.open");
    let api::HostMessage::Request { id, .. } = next(&mut requester) else {
        panic!()
    };
    let context = host.app.plugins.instances[&0].application.requests[&id].clone();
    let revision = host.app.buffers[index].revision();
    // Exercise the native coordinator directly to inspect its typed admission error.
    let error = host
        .start_provider_write_with_context(1, index, None, Some(context))
        .unwrap_err();
    assert_eq!(error.code, api::ErrorCode::LimitExceeded);
    assert!(error.message.contains("4096"));
    assert!(error.message.contains("trim"));
    assert_eq!(host.app.buffers[index].revision(), revision);
    assert!(host.provider_writes.is_empty());
    let state = &host.app.plugins.instances[&1].application;
    assert!(state.jobs.is_empty());
    assert!(state.buffers.is_empty());
    assert_eq!(state.retained_payload, 0);
    assert!(!host.app.has_input_overlay());
    assert_no_resource(&mut provider);
}
