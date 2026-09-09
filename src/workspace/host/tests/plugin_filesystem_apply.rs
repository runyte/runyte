// SPDX-License-Identifier: MPL-2.0

use super::filesystem::{
    applied, applied_event, complete, finished, foreground, list, local_service, response, started,
};
use super::*;
use crate::input::{InputEvent, KeyStroke};
use crate::plugin::filesystem::Intent;

pub(super) async fn confirmation(
    host: &mut WorkspaceHost,
    events: &mut mpsc::Receiver<Event>,
    output: &mut mpsc::Receiver<HostMessage>,
    operation: &str,
) -> String {
    let (directory, revision, entries) = list(host, events, output, 1).await;
    let intent = if operation == "create" {
        Intent::CreateFile {
            destination: "created.txt".into(),
        }
    } else {
        let entry = entries
            .iter()
            .find(|entry| entry.name == "source.txt")
            .unwrap()
            .entry
            .clone();
        if operation == "rename" {
            Intent::Rename {
                entry,
                destination: "renamed.txt".into(),
            }
        } else {
            Intent::Trash { entry }
        }
    };
    request(
        host,
        0,
        2,
        api::Request::FilesystemPrepare {
            directory,
            expected_revision: revision,
            intent,
        },
    );
    let api::ResultValue::FilesystemPlan { plan, .. } =
        complete(host, events, output).await.unwrap()
    else {
        panic!()
    };
    let invocation = foreground(host, output);
    request(
        host,
        0,
        3,
        api::Request::FilesystemApply {
            plan: plan.clone(),
            invocation,
        },
    );
    response(output).unwrap();
    assert!(host.app.fs_confirmation.is_some());
    plan
}
pub(super) fn accept(
    host: &mut WorkspaceHost,
    output: &mut mpsc::Receiver<HostMessage>,
) -> (String, String) {
    host.app
        .handle_key(KeyStroke::parse("Enter").unwrap())
        .unwrap();
    host.sync_plugin_observers();
    assert!(host.app.fs_confirmation.is_none());
    assert!(host.app.plugins.filesystem_applying);
    started(output)
}
fn edit(host: &mut WorkspaceHost, buffer: usize, value: &str) {
    host.apply_expected_transaction(
        BufferId::from_index(buffer),
        BufferRevision::from_raw(host.app.buffers[buffer].revision()),
        Transaction::insert(0, value),
    )
    .unwrap();
}

#[tokio::test]
async fn queued_filesystem_completion_preserves_newer_text_and_reconciles_renamed_identity() {
    let (root, mut host) = host();
    let source = root.path().join("source.txt");
    std::fs::write(&source, "baseline é\n").unwrap();
    let buffer = host.app.host_open_file(source.clone(), true).unwrap();
    let mut output = setup(&mut host, 0, &["filesystem", "jobs"]);
    next(&mut output);
    let mut events = local_service(&mut host);
    let plan = confirmation(&mut host, &mut events, &mut output, "rename").await;
    let (started_plan, _) = accept(&mut host, &mut output);
    assert_eq!(started_plan, plan);
    let event = applied_event(&mut events).await;
    assert_eq!(host.app.buffers[buffer].path.as_ref(), Some(&source));
    edit(&mut host, buffer, "newer ");
    host.handle_plugin_event(event);
    assert_eq!(finished(&mut output).state, "succeeded");
    assert!(!host.app.plugins.filesystem_applying);
    assert_eq!(
        host.app.buffers[buffer].path.as_ref(),
        Some(&root.path().join("renamed.txt"))
    );
    assert_eq!(host.app.buffers[buffer].to_string(), "newer baseline é\n");
    assert!(host.app.buffers[buffer].dirty);
    assert_eq!(
        std::fs::read_to_string(root.path().join("renamed.txt")).unwrap(),
        "baseline é\n"
    );
}

#[tokio::test]
async fn filesystem_apply_barrier_protects_documents_waits_and_quit_but_allows_detach() {
    let (root, mut host) = host();
    let source = root.path().join("source.txt");
    std::fs::write(&source, "baseline\n").unwrap();
    let (wait, buffers) = host.create_wait_request([source], true).unwrap();
    let buffer = buffers[0];
    let index = buffer.index().unwrap();
    let mut output = setup(&mut host, 0, &["filesystem", "jobs", "documents"]);
    next(&mut output);
    let mut events = local_service(&mut host);
    confirmation(&mut host, &mut events, &mut output, "rename").await;
    accept(&mut host, &mut output);
    let event = applied_event(&mut events).await;
    assert!(!host.app.buffers[index].dirty);
    for discard in [false, true] {
        assert!(host.close_buffer(buffer, discard).is_err());
    }
    assert!(host.complete_wait_buffer(wait, buffer).is_err());
    assert!(host.complete_wait_request(wait).is_err());
    for command in ["q", "q!", "qa", "qa!", "close", "close!", "reload"] {
        let invocation = host.app.parse_command(command).unwrap();
        let _ = host.app.execute(invocation);
        assert!(!host.app.should_quit, "{command}");
        assert!(!host.app.host_buffer_is_closed(index), "{command}");
    }
    assert_eq!(host.protected_state().plugin_jobs, 1);
    assert!(!host.may_retire_idle());
    host.app.enable_persistent_session();
    invoke(&mut host, "detach");
    assert!(host.app.should_quit);
    host.app.note_plugin_frontend(false);
    host.sync_plugin_observers();
    assert!(host.app.plugins.filesystem_applying);
    host.handle_plugin_event(event);
    assert_eq!(finished(&mut output).state, "succeeded");
    assert!(!host.app.plugins.filesystem_applying);
    host.complete_wait_request(wait).unwrap();
}

#[tokio::test]
async fn stopped_filesystem_owner_remains_protected_until_queued_result_reconciles() {
    let (root, mut host) = host();
    let source = root.path().join("source.txt");
    std::fs::write(&source, "baseline\n").unwrap();
    let buffer = host.app.host_open_file(source, true).unwrap();
    let mut output = setup(&mut host, 0, &["filesystem", "jobs"]);
    next(&mut output);
    let mut events = local_service(&mut host);
    confirmation(&mut host, &mut events, &mut output, "rename").await;
    accept(&mut host, &mut output);
    let event = applied_event(&mut events).await;
    host.stop_plugin(0, "test owner stopped");
    assert!(host.app.plugins.filesystem_applying);
    assert!(host.app.plugins.orphaned_payload > 0);
    assert_eq!(host.protected_state().plugin_jobs, 1);
    assert!(!host.may_retire_idle());
    assert!(
        host.close_buffer(BufferId::from_index(buffer), true)
            .is_err()
    );
    edit(&mut host, buffer, "retained ");
    host.handle_plugin_event(event);
    assert!(!host.app.plugins.filesystem_applying);
    assert_eq!(host.app.plugins.orphaned_payload, 0);
    assert_eq!(host.protected_state().plugin_jobs, 0);
    assert_eq!(
        host.app.buffers[buffer].path.as_ref(),
        Some(&root.path().join("renamed.txt"))
    );
    assert_eq!(host.app.buffers[buffer].to_string(), "retained baseline\n");
    assert!(output.try_recv().is_err());
}

#[tokio::test]
async fn saturated_filesystem_worker_slots_leave_confirmation_available_to_retry() {
    let (root, mut host) = host();
    let mut output = setup(&mut host, 0, &["filesystem", "jobs"]);
    next(&mut output);
    let mut events = local_service(&mut host);
    let plan = confirmation(&mut host, &mut events, &mut output, "create").await;
    let slots = std::sync::Arc::new(tokio::sync::Semaphore::new(0));
    host.plugin_local_slots = Some(slots.clone());
    host.app
        .handle_key(KeyStroke::parse("Enter").unwrap())
        .unwrap();
    host.sync_plugin_observers();
    assert!(host.app.fs_confirmation.is_some());
    assert_eq!(
        host.app.plugins.filesystem_confirmation.as_ref().unwrap().1,
        plan
    );
    assert!(!host.app.plugins.filesystem_applying);
    assert!(host.app.plugins.instances[&0].application.jobs.is_empty());
    assert!(!root.path().join("created.txt").exists());
    slots.add_permits(1);
    accept(&mut host, &mut output);
    assert_eq!(
        applied(&mut host, &mut events, &mut output).await.state,
        "succeeded"
    );
    assert!(root.path().join("created.txt").is_file());
}

#[tokio::test]
async fn filesystem_apply_keeps_scratch_and_generated_buffer_retirement_responsive() {
    let (_root, mut host) = host();
    let mut output = setup(&mut host, 0, &["filesystem", "jobs"]);
    next(&mut output);
    let mut events = local_service(&mut host);
    confirmation(&mut host, &mut events, &mut output, "create").await;
    accept(&mut host, &mut output);
    let event = applied_event(&mut events).await;
    let first_scratch = host.app.active().buffer;
    invoke(&mut host, "new");
    assert!(host.app.host_buffer_is_closed(first_scratch));
    let mut generated = Vec::new();
    for n in 0..20 {
        generated.push(host.app.create_plugin_view(
            0,
            &format!("retention:{n}"),
            &model(&[("row", "generated")]),
        ));
    }
    assert!(host.app.host_buffer_is_closed(generated[0]));
    assert!(
        generated
            .iter()
            .any(|index| !host.app.host_buffer_is_closed(*index))
    );
    assert!(host.app.plugins.filesystem_applying);
    host.handle_plugin_event(event);
    assert_eq!(finished(&mut output).state, "succeeded");
}

#[tokio::test]
async fn filesystem_apply_refuses_new_native_and_late_plugin_document_publication() {
    let (root, mut host) = host();
    let source = root.path().join("source.txt");
    let late = root.path().join("late.txt");
    std::fs::write(&source, "baseline\n").unwrap();
    std::fs::write(&late, "late\n").unwrap();
    let buffer = host.app.host_open_file(source.clone(), true).unwrap();
    let mut output = setup(&mut host, 0, &["filesystem", "jobs", "documents"]);
    next(&mut output);
    let mut events = local_service(&mut host);
    confirmation(&mut host, &mut events, &mut output, "rename").await;
    request(
        &mut host,
        0,
        4,
        api::Request::BufferOpen {
            path: "late.txt".into(),
            invocation: None,
        },
    );
    let late_event = tokio::time::timeout(std::time::Duration::from_secs(3), events.recv())
        .await
        .unwrap()
        .unwrap();
    accept(&mut host, &mut output);
    assert!(host.open_buffer(late.clone(), false).is_err());
    let count = host.app.buffers.len();
    host.handle_plugin_event(late_event);
    assert_eq!(
        response(&mut output).unwrap_err().code,
        api::ErrorCode::Busy
    );
    assert_eq!(host.app.buffers.len(), count);
    assert_eq!(host.app.buffers[buffer].path.as_ref(), Some(&source));
    assert_eq!(
        applied(&mut host, &mut events, &mut output).await.state,
        "succeeded"
    );
    assert!(host.open_buffer(late, false).is_ok());
}

struct ReleaseTrash(std::sync::Arc<(std::sync::Mutex<bool>, std::sync::Condvar)>);
impl Drop for ReleaseTrash {
    fn drop(&mut self) {
        *self.0.0.lock().unwrap() = true;
        self.0.1.notify_all();
    }
}
struct BlockingTrash {
    entered: std::sync::Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
    release: std::sync::Arc<(std::sync::Mutex<bool>, std::sync::Condvar)>,
}
impl crate::fs_plan::TrashBackend for BlockingTrash {
    fn delete(&self, path: &std::path::Path) -> anyhow::Result<()> {
        let _ = self.entered.lock().unwrap().take().unwrap().send(());
        let (released, timeout) = self
            .release
            .1
            .wait_timeout_while(
                self.release.0.lock().unwrap(),
                std::time::Duration::from_secs(5),
                |released| !*released,
            )
            .unwrap();
        anyhow::ensure!(
            *released && !timeout.timed_out(),
            "test did not release trash worker"
        );
        std::fs::remove_file(path)?;
        Ok(())
    }
}

#[tokio::test]
async fn blocked_trash_worker_leaves_input_responsive_and_preserves_newer_deleted_text() {
    let (root, mut host) = host();
    let source = root.path().join("source.txt");
    std::fs::write(&source, "baseline\n").unwrap();
    let buffer = host.app.host_open_file(source.clone(), true).unwrap();
    let (entered, waiting) = tokio::sync::oneshot::channel();
    let release = ReleaseTrash(std::sync::Arc::new((
        std::sync::Mutex::new(false),
        std::sync::Condvar::new(),
    )));
    host.app.set_trash_backend(Box::new(BlockingTrash {
        entered: std::sync::Mutex::new(Some(entered)),
        release: release.0.clone(),
    }));
    let mut output = setup(&mut host, 0, &["filesystem", "jobs"]);
    next(&mut output);
    let mut events = local_service(&mut host);
    confirmation(&mut host, &mut events, &mut output, "trash").await;
    accept(&mut host, &mut output);
    tokio::time::timeout(std::time::Duration::from_secs(3), waiting)
        .await
        .unwrap()
        .unwrap();
    host.app.handle_key(KeyStroke::char('i')).unwrap();
    host.app
        .handle_input(InputEvent::Text("typed while deleting ".into()))
        .unwrap();
    assert_eq!(
        host.app.buffers[buffer].to_string(),
        "typed while deleting baseline\n"
    );
    assert!(events.try_recv().is_err());
    drop(release);
    assert_eq!(
        applied(&mut host, &mut events, &mut output).await.state,
        "succeeded"
    );
    assert!(!source.exists());
    assert_eq!(
        host.app.buffers[buffer].to_string(),
        "typed while deleting baseline\n"
    );
    assert!(host.app.buffers[buffer].dirty);
    assert_eq!(host.app.buffers[buffer].path.as_ref(), Some(&source));
    assert_eq!(
        host.app.buffers[buffer].external_file_status(),
        crate::buffer::ExternalFileStatus::Deleted
    );
}
