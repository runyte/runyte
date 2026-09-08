// SPDX-License-Identifier: MPL-2.0

use super::*;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

fn pending_document_job(
    host: &mut WorkspaceHost,
    output: &mut mpsc::Receiver<HostMessage>,
) -> (String, Arc<AtomicBool>) {
    let (api::ResultValue::Job(job), _) = host
        .application_request(
            0,
            api::Request::JobCreate {
                title: "Save document".into(),
                deadline_seconds: 60,
            },
        )
        .unwrap()
    else {
        panic!()
    };
    assert!(matches!(
        output.try_recv().unwrap(),
        HostMessage::Deadline {
            after_ms: Some(60000),
            ..
        }
    ));
    let state = &mut host.app.plugins.instances.get_mut(&0).unwrap().application;
    state.retained_payload = 16 * 1024 * 1024;
    let cancelled = Arc::new(AtomicBool::new(false));
    host.document_saves.insert(
        job.job.clone(),
        crate::workspace::host::plugin_documents::PendingSave {
            owner: 0,
            generation: state.generation.clone(),
            buffer: 0,
            cancelled: cancelled.clone(),
            orphaned: false,
        },
    );
    host.app.plugins.document_saves.insert(0);
    (job.job, cancelled)
}

fn expire(host: &mut WorkspaceHost, token: &str) {
    if let Some(deadline) = host
        .app
        .plugins
        .instances
        .get_mut(&0)
        .unwrap()
        .application
        .deadlines
        .get_mut(token)
    {
        *deadline = std::time::Instant::now();
    }
    host.handle_plugin_event(Event {
        plugin: 0,
        result: Ok(ClientMessage::Deadline {
            token: token.into(),
        }),
    });
}

#[test]
fn document_job_cancel_retains_protection_until_host_io_completes() {
    let (_root, mut host) = host();
    let mut output = setup(&mut host, 0, &["jobs"]);
    next(&mut output);
    let (token, cancelled) = pending_document_job(&mut host, &mut output);
    request(
        &mut host,
        0,
        1,
        api::Request::JobCancel { job: token.clone() },
    );
    assert!(matches!(
        output.try_recv().unwrap(),
        HostMessage::Deadline { after_ms: None, .. }
    ));
    assert_eq!(job(&mut output).state, api::JobState::Cancelling);
    assert!(matches!(
        next(&mut output),
        api::HostMessage::Event {
            event: "job.changed",
            ..
        }
    ));
    assert!(cancelled.load(Ordering::SeqCst));
    assert_eq!(host.protected_state().plugin_jobs, 1);
    assert!(!host.may_retire_idle());
    assert!(host.close_buffer(BufferId::from_index(0), true).is_err());

    // Duplicate cancellation and a previously queued deadline are idempotent.
    request(
        &mut host,
        0,
        2,
        api::Request::JobCancel { job: token.clone() },
    );
    assert_eq!(job(&mut output).state, api::JobState::Cancelling);
    expire(&mut host, &token);
    expire(&mut host, &token);
    assert!(host.app.plugins.instances.contains_key(&0));
    assert!(output.try_recv().is_err());
    assert!(host.document_saves.contains_key(&token));

    host.complete_document_save(token.clone(), Ok(None));
    assert!(matches!(
        output.try_recv().unwrap(),
        HostMessage::Deadline { after_ms: None, .. }
    ));
    assert!(matches!(
        next(&mut output),
        api::HostMessage::Event {
            event: "job.changed",
            data: api::EventData::Job(api::Job {
                state: api::JobState::Cancelled,
                ..
            }),
            ..
        }
    ));
    assert_eq!(host.protected_state().plugin_jobs, 0);
    assert!(host.app.plugins.document_saves.is_empty());
    assert_eq!(
        host.app.plugins.instances[&0].application.retained_payload,
        0
    );
    expire(&mut host, &token);
    assert!(host.app.plugins.instances.contains_key(&0));
}

#[test]
fn document_job_deadline_cancels_host_work_without_plugin_acknowledgement() {
    let (_root, mut host) = host();
    let mut output = setup(&mut host, 0, &["jobs"]);
    next(&mut output);
    let (token, cancelled) = pending_document_job(&mut host, &mut output);
    expire(&mut host, &token);
    assert!(matches!(
        next(&mut output),
        api::HostMessage::Event {
            event: "job.changed",
            data: api::EventData::Job(api::Job {
                state: api::JobState::Cancelling,
                ..
            }),
            ..
        }
    ));
    assert!(matches!(
        output.try_recv().unwrap(),
        HostMessage::Deadline { after_ms: None, .. }
    ));
    assert!(cancelled.load(Ordering::SeqCst));
    expire(&mut host, &token);
    assert!(host.app.plugins.instances.contains_key(&0));
    request(&mut host, 0, 1, api::Request::JobGet { job: token.clone() });
    assert_eq!(job(&mut output).state, api::JobState::Cancelling);
    assert_eq!(host.protected_state().plugin_jobs, 1);
    assert!(output.try_recv().is_err());
}

#[test]
fn document_job_cancellation_and_completion_enforce_owner_authority() {
    let (_root, mut host) = host();
    let mut output = setup(&mut host, 0, &["jobs"]);
    let mut foreign = setup(&mut host, 1, &["jobs"]);
    next(&mut output);
    next(&mut foreign);
    let (token, cancelled) = pending_document_job(&mut host, &mut output);
    let error = host
        .application_request(1, api::Request::JobCancel { job: token.clone() })
        .unwrap_err();
    assert_eq!(error.code, api::ErrorCode::NotFound);
    assert!(!cancelled.load(Ordering::SeqCst));
    for request in [
        api::Request::JobUpdate {
            job: token.clone(),
            progress: 100,
        },
        api::Request::JobFinish {
            job: token.clone(),
            state: api::TerminalState::Succeeded,
        },
    ] {
        assert_eq!(
            host.application_request(0, request).unwrap_err().code,
            api::ErrorCode::Conflict
        );
    }
    host.app
        .plugins
        .instances
        .get_mut(&0)
        .unwrap()
        .application
        .capabilities
        .remove("jobs");
    assert_eq!(
        host.application_request(0, api::Request::JobCancel { job: token.clone() })
            .unwrap_err()
            .code,
        api::ErrorCode::CapabilityDenied
    );
    assert!(!cancelled.load(Ordering::SeqCst));
    assert_eq!(host.protected_state().plugin_jobs, 1);
}

#[tokio::test]
async fn known_stale_document_save_refuses_before_whitespace_hook() {
    let (root, mut host) = host();
    let path = root.path().join("stale.txt");
    std::fs::write(&path, "old\n").unwrap();
    let buffer = host
        .open_buffer(path.clone(), false)
        .unwrap()
        .index()
        .unwrap();
    host.apply_expected_transaction(
        BufferId::from_index(buffer),
        BufferRevision::from_raw(host.app.buffers[buffer].revision()),
        Transaction::insert(0, "local \t\n"),
    )
    .unwrap();
    std::fs::write(&path, "external\n").unwrap();
    let observation = host.app.buffers[buffer].observe_now(buffer).unwrap();
    host.app.apply_file_observation(observation);
    assert!(host.app.buffers[buffer].external_file_status().is_stale());
    let text = host.app.buffers[buffer].to_string();
    let revision = host.app.buffers[buffer].revision();
    let history = host.app.buffers[buffer].history_len();
    let mut output = setup(&mut host, 0, &["documents", "jobs"]);
    next(&mut output);
    let mut events = super::filesystem::local_service(&mut host);
    let handle = host
        .app
        .plugins
        .instances
        .get_mut(&0)
        .unwrap()
        .application
        .buffer_handle(buffer)
        .unwrap();
    request(
        &mut host,
        0,
        1,
        api::Request::BufferSave {
            buffer: handle,
            expected_revision: format!("r:{revision}"),
        },
    );
    assert_eq!(
        super::filesystem::response(&mut output).unwrap_err().code,
        api::ErrorCode::Conflict
    );
    assert_eq!(host.app.buffers[buffer].to_string(), text);
    assert_eq!(host.app.buffers[buffer].revision(), revision);
    assert_eq!(host.app.buffers[buffer].history_len(), history);
    assert_eq!(std::fs::read_to_string(path).unwrap(), "external\n");
    assert!(host.document_saves.is_empty());
    assert!(host.app.plugins.instances[&0].application.jobs.is_empty());
    assert!(events.try_recv().is_err());
}

#[test]
fn newly_created_documents_have_no_clean_empty_baseline_before_first_save() {
    for initial in ["", "new text"] {
        let (root, _) = host();
        let path = root.path().join("new.txt");
        let mut buffer = crate::buffer::Buffer::unsaved_document(path.clone(), initial.into());
        assert!(buffer.dirty);
        assert!(buffer.holds_unsaved_work());
        if !initial.is_empty() {
            buffer.undo();
            assert_eq!(buffer.to_string(), "");
            assert!(buffer.dirty);
        }
        assert!(!path.exists());
        let saved = buffer
            .prepare_document_save()
            .unwrap()
            .run(root.path())
            .unwrap();
        assert!(buffer.accept_document_save(saved));
        assert_eq!(std::fs::read_to_string(path).unwrap(), "");
        assert!(!buffer.dirty);
        buffer.apply(&Transaction::insert(0, "after save"));
        assert!(buffer.dirty);
        buffer.undo();
        assert!(!buffer.dirty);
    }
}

#[test]
fn completed_document_write_retains_recovery_warning_even_when_cancelled() {
    for cancelled in [false, true] {
        let (root, mut host) = host();
        let path = root.path().join("written.txt");
        std::fs::write(&path, "text\n").unwrap();
        let buffer = host.open_buffer(path, false).unwrap().index().unwrap();
        let mut saved = host.app.buffers[buffer]
            .prepare_document_save()
            .unwrap()
            .run(root.path())
            .unwrap();
        let recovery = root.path().join("retained-backup");
        let detail = format!("Displaced contents remain at {}", recovery.display());
        saved.warning = Some(detail.clone());
        assert_eq!(
            host.app
                .finish_plugin_document_save(buffer, cancelled, Ok(Some(saved))),
            if cancelled {
                api::JobState::OutcomeUnknown
            } else {
                api::JobState::Succeeded
            }
        );
        assert!(
            host.app
                .notifications()
                .entries()
                .iter()
                .any(|entry| entry.body.contains(&detail)),
            "actionable recovery warning must survive cancellation={cancelled}"
        );
        assert_eq!(host.app.buffers[buffer].dirty, cancelled);
    }
}

#[test]
fn new_wait_reuses_pending_document_without_refreshing_its_save_baseline() {
    let (root, mut host) = host();
    let path = root.path().join("wait.txt");
    std::fs::write(&path, "text\n").unwrap();
    host.app.buffers[0] = crate::buffer::Buffer::open(&path).unwrap();
    let work = host.app.buffers[0].prepare_document_save().unwrap();
    let baseline = host.app.buffers[0].file_observation_request(0).unwrap();
    let mut output = setup(&mut host, 0, &["jobs"]);
    next(&mut output);
    let (token, _) = pending_document_job(&mut host, &mut output);
    let saved = work.run(root.path()).unwrap();
    let (wait, buffers) = host.create_wait_request([path], false).unwrap();
    assert_eq!(buffers, [BufferId::from_index(0)]);
    assert_eq!(
        host.app.buffers[0].file_observation_request(0).unwrap(),
        baseline
    );
    assert!(host.complete_wait_request(wait).is_err());
    host.complete_document_save(token, Ok(Some(saved)));
    assert!(!host.app.buffers[0].dirty);
    host.complete_wait_request(wait).unwrap();
    assert_eq!(
        host.wait_status(wait),
        Some(crate::workspace::WaitStatus::Completed)
    );
}
