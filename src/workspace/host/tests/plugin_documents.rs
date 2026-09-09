// SPDX-License-Identifier: MPL-2.0
use super::filesystem::{complete, local_service, response};
use super::*;

async fn opened(
    host: &mut WorkspaceHost,
    events: &mut mpsc::Receiver<Event>,
    output: &mut mpsc::Receiver<HostMessage>,
    create: bool,
) -> (String, usize) {
    request(
        host,
        0,
        1,
        if create {
            api::Request::BufferCreate {
                path: "document.txt".into(),
                text: "new é\n".into(),
                invocation: None,
            }
        } else {
            api::Request::BufferOpen {
                path: "document.txt".into(),
                invocation: None,
            }
        },
    );
    let api::ResultValue::Opened { buffer, .. } = complete(host, events, output).await.unwrap()
    else {
        panic!()
    };
    let index = host.app.plugins.instances[&0].application.buffers[&buffer];
    (buffer, index)
}
fn save(
    host: &mut WorkspaceHost,
    output: &mut mpsc::Receiver<HostMessage>,
    n: u64,
    handle: &str,
    index: usize,
) -> api::Job {
    request(
        host,
        0,
        n,
        api::Request::BufferSave {
            buffer: handle.into(),
            expected_revision: format!("r:{}", host.app.buffers[index].revision()),
        },
    );
    let job = job(output);
    assert!(matches!(
        next(output),
        api::HostMessage::Event {
            event: "job.changed",
            ..
        }
    ));
    job
}
async fn saved_event(events: &mut mpsc::Receiver<Event>) -> Event {
    tokio::time::timeout(std::time::Duration::from_secs(3), events.recv())
        .await
        .unwrap()
        .unwrap()
}
fn edit(host: &mut WorkspaceHost, index: usize, text: &str) {
    host.apply_expected_transaction(
        BufferId::from_index(index),
        BufferRevision::from_raw(host.app.buffers[index].revision()),
        Transaction::insert(0, text),
    )
    .unwrap();
}
fn finished(output: &mut mpsc::Receiver<HostMessage>, state: api::JobState) {
    assert!(
        matches!(next(output), api::HostMessage::Event { data: api::EventData::Job(job), .. } if job.state == state)
    );
}
#[tokio::test]
async fn save_keeps_newer_edits_and_undo_returns_to_captured_baseline() {
    let (root, mut host) = host();
    let path = root.path().join("document.txt");
    std::fs::write(&path, "disk\n").unwrap();
    let mut output = setup(&mut host, 0, &["documents", "jobs"]);
    next(&mut output);
    let mut events = local_service(&mut host);
    let (handle, index) = opened(&mut host, &mut events, &mut output, false).await;
    edit(&mut host, index, "é猫 ");
    save(&mut host, &mut output, 2, &handle, index);
    let event = saved_event(&mut events).await;
    // A file watcher can report the installed text before IO completion is drained.
    let observation = host.app.buffers[index].observe_now(index).unwrap();
    host.app.apply_file_observation(observation);
    edit(&mut host, index, "newer ");
    host.handle_plugin_event(event);
    finished(&mut output, api::JobState::Succeeded);
    assert_eq!(std::fs::read_to_string(path).unwrap(), "é猫 disk\n");
    assert_eq!(host.app.buffers[index].to_string(), "newer é猫 disk\n");
    assert!(host.app.buffers[index].dirty);
    host.app.buffers[index].undo();
    assert_eq!(host.app.buffers[index].to_string(), "é猫 disk\n");
    assert!(!host.app.buffers[index].dirty);
    assert!(host.document_saves.is_empty());
    assert_eq!(
        host.app.plugins.instances[&0].application.retained_payload,
        0
    );
}
#[tokio::test]
async fn save_runs_normal_whitespace_hook_and_rejects_stale_and_pending_operations() {
    let (root, mut host) = host();
    let path = root.path().join("document.txt");
    std::fs::write(&path, "disk\n").unwrap();
    let mut output = setup(&mut host, 0, &["documents", "jobs"]);
    next(&mut output);
    let mut events = local_service(&mut host);
    let (handle, index) = opened(&mut host, &mut events, &mut output, false).await;
    edit(&mut host, index, "é \t\n");
    request(
        &mut host,
        0,
        2,
        api::Request::BufferSave {
            buffer: handle.clone(),
            expected_revision: "r:0".into(),
        },
    );
    assert_eq!(
        response(&mut output).unwrap_err().code,
        api::ErrorCode::Stale
    );
    assert!(host.app.buffers[index].to_string().contains('\t'));
    save(&mut host, &mut output, 3, &handle, index);
    let revision = format!("r:{}", host.app.buffers[index].revision());
    for (n, operation) in [
        api::Request::BufferSave {
            buffer: handle.clone(),
            expected_revision: revision.clone(),
        },
        api::Request::BufferClose {
            buffer: handle.clone(),
            expected_revision: revision.clone(),
        },
    ]
    .into_iter()
    .enumerate()
    {
        request(&mut host, 0, 4 + n as u64, operation);
        assert_eq!(
            response(&mut output).unwrap_err().code,
            api::ErrorCode::Busy
        );
    }
    assert!(host.app.host_close_buffer(index, false).is_err());
    host.handle_plugin_event(saved_event(&mut events).await);
    finished(&mut output, api::JobState::Succeeded);
    assert_eq!(std::fs::read_to_string(path).unwrap(), "é\ndisk\n");
    assert!(!host.app.buffers[index].dirty);
    host.app.buffers[index].undo();
    assert!(host.app.buffers[index].to_string().contains('\t'));
    assert!(host.app.buffers[index].dirty);
}
#[tokio::test]
async fn create_is_unsaved_until_save_and_never_overwrites_a_new_disk_file() {
    for collision in [false, true] {
        let (root, mut host) = host();
        let path = root.path().join("document.txt");
        let mut output = setup(&mut host, 0, &["documents", "jobs"]);
        next(&mut output);
        let mut events = local_service(&mut host);
        let (handle, index) = opened(&mut host, &mut events, &mut output, true).await;
        assert!(!path.exists());
        assert!(host.app.buffers[index].dirty);
        if collision {
            std::fs::write(&path, "external").unwrap();
        }
        save(&mut host, &mut output, 2, &handle, index);
        host.handle_plugin_event(saved_event(&mut events).await);
        finished(
            &mut output,
            if collision {
                api::JobState::Failed
            } else {
                api::JobState::Succeeded
            },
        );
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            if collision { "external" } else { "new é\n" }
        );
        let expected_revision = format!("r:{}", host.app.buffers[index].revision());
        request(
            &mut host,
            0,
            3,
            api::Request::BufferClose {
                buffer: handle,
                expected_revision,
            },
        );
        if collision {
            assert_eq!(
                response(&mut output).unwrap_err().code,
                api::ErrorCode::Conflict
            );
        } else {
            response(&mut output).unwrap();
            assert!(host.app.host_buffer_is_closed(index));
        }
    }
}
#[tokio::test]
async fn cancellation_and_owner_stop_preserve_uncertain_text_until_explicit_reconciliation() {
    for stop in [false, true] {
        let (root, mut host) = host();
        let path = root.path().join("document.txt");
        std::fs::write(&path, "disk\n").unwrap();
        let mut output = setup(&mut host, 0, &["documents", "jobs"]);
        next(&mut output);
        let mut events = local_service(&mut host);
        let (handle, index) = opened(&mut host, &mut events, &mut output, false).await;
        edit(&mut host, index, "saved ");
        let job = save(&mut host, &mut output, 2, &handle, index);
        let event = saved_event(&mut events).await;
        if stop {
            host.stop_plugin(0, "test stop");
            assert!(host.app.plugins.orphaned_payload > 0);
        } else {
            request(&mut host, 0, 3, api::Request::JobCancel { job: job.job });
            response(&mut output).unwrap();
            next(&mut output);
        }
        assert!(host.app.plugins.document_saves.contains(&index));
        host.handle_plugin_event(event);
        if !stop {
            finished(&mut output, api::JobState::OutcomeUnknown);
        }
        assert_eq!(std::fs::read_to_string(path).unwrap(), "saved disk\n");
        let observation = host.app.buffers[index].observe_now(index).unwrap();
        host.app.apply_file_observation(observation);
        assert!(host.app.buffers[index].dirty);
        host.app.buffers[index].undo();
        assert!(host.app.buffers[index].dirty);
        assert!(host.app.plugins.document_saves.is_empty());
        assert_eq!(host.app.plugins.orphaned_payload, 0);
    }
}
