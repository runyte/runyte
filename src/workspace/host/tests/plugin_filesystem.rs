// SPDX-License-Identifier: MPL-2.0

use super::*;
use crate::input::KeyStroke;
use crate::plugin::filesystem::Intent;

fn local_service(host: &mut WorkspaceHost) -> mpsc::Receiver<Event> {
    let (sender, receiver) = mpsc::channel(16);
    host.plugin_events_sender = Some(sender);
    receiver
}

async fn complete(
    host: &mut WorkspaceHost,
    events: &mut mpsc::Receiver<Event>,
    output: &mut mpsc::Receiver<HostMessage>,
) -> Result<api::ResultValue, api::Error> {
    let event = tokio::time::timeout(std::time::Duration::from_secs(3), events.recv())
        .await
        .unwrap()
        .unwrap();
    host.handle_plugin_event(event);
    response(output)
}
fn response(output: &mut mpsc::Receiver<HostMessage>) -> Result<api::ResultValue, api::Error> {
    match next(output) {
        api::HostMessage::Response {
            outcome: api::Response::Success { result },
            ..
        } => Ok(result),
        api::HostMessage::Response {
            outcome: api::Response::Failure { error },
            ..
        } => Err(error),
        other => panic!("unexpected {other:?}"),
    }
}
async fn list(
    host: &mut WorkspaceHost,
    events: &mut mpsc::Receiver<Event>,
    output: &mut mpsc::Receiver<HostMessage>,
    serial: u64,
) -> (String, String, Vec<crate::plugin::filesystem::Entry>) {
    request(
        host,
        0,
        serial,
        api::Request::FilesystemList {
            path: ".".into(),
            offset: 0,
            limit: 128,
            expected_revision: None,
        },
    );
    let api::ResultValue::Directory {
        directory,
        revision,
        entries,
        ..
    } = complete(host, events, output).await.unwrap()
    else {
        panic!()
    };
    (directory, revision, entries)
}
fn foreground(host: &mut WorkspaceHost, output: &mut mpsc::Receiver<HostMessage>) -> String {
    host.app.note_plugin_frontend(true);
    invoke(host, "plugin.app-0.open");
    let api::HostMessage::Request { id, .. } = next(output) else {
        panic!()
    };
    id
}

#[tokio::test]
async fn local_directory_pages_are_revision_bound_owned_and_releasable() {
    let (root, mut host) = host();
    std::fs::write(root.path().join("é.txt"), "hello").unwrap();
    let mut output = setup(&mut host, 0, &["filesystem"]);
    let mut other = setup(&mut host, 1, &["filesystem"]);
    next(&mut output);
    next(&mut other);
    let mut events = local_service(&mut host);
    let (directory, revision, entries) = list(&mut host, &mut events, &mut output, 1).await;
    assert!(
        entries
            .iter()
            .any(|e| e.name == "é.txt" && e.bytes == 5 && e.kind == "file")
    );
    std::fs::write(root.path().join("new.txt"), "changed").unwrap();
    request(
        &mut host,
        0,
        2,
        api::Request::FilesystemList {
            path: ".".into(),
            offset: 1,
            limit: 1,
            expected_revision: Some(revision.clone()),
        },
    );
    assert_eq!(
        complete(&mut host, &mut events, &mut output)
            .await
            .unwrap_err()
            .code,
        api::ErrorCode::Stale
    );
    request(
        &mut host,
        1,
        1,
        api::Request::FilesystemPrepare {
            directory: directory.clone(),
            expected_revision: revision,
            intent: Intent::CreateFile {
                destination: "wrong".into(),
            },
        },
    );
    assert_eq!(
        response(&mut other).unwrap_err().code,
        api::ErrorCode::NotFound
    );
    request(
        &mut host,
        0,
        3,
        api::Request::FilesystemRelease {
            directory: directory.clone(),
        },
    );
    response(&mut output).unwrap();
    request(
        &mut host,
        0,
        4,
        api::Request::FilesystemRelease { directory },
    );
    response(&mut output).unwrap();
    assert_eq!(
        host.app.plugins.instances[&0].application.retained_payload,
        0
    );
}

#[tokio::test]
async fn local_mutations_require_real_confirmation_and_revalidate_collisions() {
    let (root, mut host) = host();
    let mut output = setup(&mut host, 0, &["filesystem"]);
    next(&mut output);
    let mut events = local_service(&mut host);
    let (directory, revision, _) = list(&mut host, &mut events, &mut output, 1).await;
    request(
        &mut host,
        0,
        2,
        api::Request::FilesystemPrepare {
            directory,
            expected_revision: revision,
            intent: Intent::CreateFile {
                destination: "created.txt".into(),
            },
        },
    );
    let api::ResultValue::FilesystemPlan { plan, .. } =
        complete(&mut host, &mut events, &mut output).await.unwrap()
    else {
        panic!()
    };
    let invocation = foreground(&mut host, &mut output);
    request(
        &mut host,
        0,
        3,
        api::Request::FilesystemApply {
            plan: plan.clone(),
            invocation,
        },
    );
    response(&mut output).unwrap();
    assert!(host.app.fs_confirmation.is_some());
    assert!(!root.path().join("created.txt").exists());
    host.app
        .handle_key(KeyStroke::parse("Esc").unwrap())
        .unwrap();
    host.sync_plugin_observers();
    assert!(
        matches!(next(&mut output), api::HostMessage::Event { event: "filesystem.finished", data: api::EventData::FilesystemFinished(value), .. } if value.state == "cancelled" && value.plan == plan)
    );
    assert!(!root.path().join("created.txt").exists());

    for (serial, collide) in [(4, false), (7, true)] {
        let (directory, revision, _) = list(&mut host, &mut events, &mut output, serial).await;
        let destination = if collide {
            "collision.txt"
        } else {
            "created.txt"
        };
        request(
            &mut host,
            0,
            serial + 1,
            api::Request::FilesystemPrepare {
                directory,
                expected_revision: revision,
                intent: Intent::CreateFile {
                    destination: destination.into(),
                },
            },
        );
        let api::ResultValue::FilesystemPlan { plan, .. } =
            complete(&mut host, &mut events, &mut output).await.unwrap()
        else {
            panic!()
        };
        let invocation = foreground(&mut host, &mut output);
        request(
            &mut host,
            0,
            serial + 2,
            api::Request::FilesystemApply { plan, invocation },
        );
        response(&mut output).unwrap();
        if collide {
            std::fs::write(root.path().join(destination), "preserve").unwrap();
        }
        host.app
            .handle_key(KeyStroke::parse("Enter").unwrap())
            .unwrap();
        host.sync_plugin_observers();
        let api::HostMessage::Event {
            data: api::EventData::FilesystemFinished(result),
            ..
        } = next(&mut output)
        else {
            panic!()
        };
        assert_eq!(result.state, if collide { "failed" } else { "succeeded" });
        assert_eq!(
            std::fs::read_to_string(root.path().join(destination)).unwrap(),
            if collide { "preserve" } else { "" }
        );
    }
}

#[tokio::test]
async fn local_copy_rename_and_trash_use_existing_plan_and_preserve_editor_text() {
    let (root, mut host) = host();
    std::fs::write(root.path().join("source.txt"), "original é").unwrap();
    let opened = host
        .app
        .host_open_file(root.path().join("source.txt"), false)
        .unwrap();
    let mut output = setup(&mut host, 0, &["filesystem"]);
    next(&mut output);
    let mut events = local_service(&mut host);
    for (index, (name, operation)) in [
        ("source.txt", "copy"),
        ("source.txt", "rename"),
        ("copy.txt", "trash"),
    ]
    .into_iter()
    .enumerate()
    {
        let serial = index as u64 * 3 + 1;
        let (directory, revision, entries) =
            list(&mut host, &mut events, &mut output, serial).await;
        let entry = entries
            .iter()
            .find(|e| e.name == name)
            .unwrap()
            .entry
            .clone();
        let intent = match operation {
            "copy" => Intent::Copy {
                entry,
                destination: "copy.txt".into(),
            },
            "rename" => Intent::Rename {
                entry,
                destination: "renamed.txt".into(),
            },
            _ => Intent::Trash { entry },
        };
        request(
            &mut host,
            0,
            serial + 1,
            api::Request::FilesystemPrepare {
                directory,
                expected_revision: revision,
                intent,
            },
        );
        let api::ResultValue::FilesystemPlan { plan, .. } =
            complete(&mut host, &mut events, &mut output).await.unwrap()
        else {
            panic!()
        };
        let invocation = foreground(&mut host, &mut output);
        request(
            &mut host,
            0,
            serial + 2,
            api::Request::FilesystemApply { plan, invocation },
        );
        response(&mut output).unwrap();
        // Permanent deletion is an explicit frontend choice; never use the person's trash in a test.
        host.app
            .handle_key(if operation == "trash" {
                KeyStroke::char('P')
            } else {
                KeyStroke::parse("Enter").unwrap()
            })
            .unwrap();
        host.sync_plugin_observers();
        assert!(
            matches!(next(&mut output), api::HostMessage::Event { data: api::EventData::FilesystemFinished(value), .. } if value.state == "succeeded")
        );
    }
    assert_eq!(
        std::fs::read_to_string(root.path().join("renamed.txt")).unwrap(),
        "original é"
    );
    assert!(!root.path().join("source.txt").exists());
    assert!(!root.path().join("copy.txt").exists());
    assert_eq!(
        host.app.buffers[opened].path.as_deref(),
        Some(root.path().join("renamed.txt").as_path())
    );
    assert_eq!(host.app.buffers[opened].to_string(), "original é");
}

#[tokio::test]
async fn local_open_reuses_unsaved_text_and_does_not_steal_changed_focus() {
    let (root, mut host) = host();
    std::fs::write(root.path().join("file.txt"), "é disk").unwrap();
    let mut output = setup(&mut host, 0, &["documents", "text"]);
    next(&mut output);
    let mut events = local_service(&mut host);
    request(
        &mut host,
        0,
        1,
        api::Request::BufferOpen {
            path: "file.txt".into(),
            invocation: None,
        },
    );
    let api::ResultValue::Opened { buffer, .. } =
        complete(&mut host, &mut events, &mut output).await.unwrap()
    else {
        panic!()
    };
    assert_eq!(host.app.active().buffer, 0);
    let index = host.app.plugins.instances[&0].application.buffers[&buffer];
    host.apply_expected_transaction(
        BufferId::from_index(index),
        BufferRevision::from_raw(host.app.buffers[index].revision()),
        Transaction::insert(0, "unsaved "),
    )
    .unwrap();
    std::fs::write(root.path().join("file.txt"), "new disk").unwrap();
    request(
        &mut host,
        0,
        2,
        api::Request::BufferOpen {
            path: "file.txt".into(),
            invocation: None,
        },
    );
    assert!(
        matches!(complete(&mut host, &mut events, &mut output).await.unwrap(), api::ResultValue::Opened { buffer: reopened, .. } if reopened == buffer)
    );
    assert_eq!(host.app.buffers[index].to_string(), "unsaved é disk");
    let invocation = foreground(&mut host, &mut output);
    request(
        &mut host,
        0,
        3,
        api::Request::BufferOpen {
            path: "file.txt".into(),
            invocation: Some(invocation),
        },
    );
    host.app.handle_key(KeyStroke::char('l')).unwrap();
    assert_eq!(
        complete(&mut host, &mut events, &mut output)
            .await
            .unwrap_err()
            .code,
        api::ErrorCode::ContextChanged
    );
    assert_eq!(host.app.active().buffer, 0);
}

#[tokio::test]
async fn local_confirmation_detach_stop_and_foreign_requests_cannot_apply() {
    let (root, mut host) = host();
    let mut output = setup(&mut host, 0, &["filesystem"]);
    next(&mut output);
    let mut events = local_service(&mut host);
    for (serial, stop) in [(1, false), (4, true)] {
        let (directory, revision, _) = list(&mut host, &mut events, &mut output, serial).await;
        request(
            &mut host,
            0,
            serial + 1,
            api::Request::FilesystemPrepare {
                directory,
                expected_revision: revision,
                intent: Intent::CreateFile {
                    destination: "never.txt".into(),
                },
            },
        );
        let api::ResultValue::FilesystemPlan { plan, .. } =
            complete(&mut host, &mut events, &mut output).await.unwrap()
        else {
            panic!()
        };
        let invocation = foreground(&mut host, &mut output);
        request(
            &mut host,
            0,
            serial + 2,
            api::Request::FilesystemApply { plan, invocation },
        );
        response(&mut output).unwrap();
        if stop {
            host.stop_plugin(0, "stopped by user");
        } else {
            host.app.note_plugin_frontend(false);
        }
        host.sync_plugin_observers();
        assert!(host.app.fs_confirmation.is_none());
        assert!(!root.path().join("never.txt").exists());
        if !stop {
            next(&mut output);
        }
    }
}

#[tokio::test]
async fn local_reads_enforce_capabilities_paths_sizes_and_inflight_limits() {
    let (root, mut host) = host();
    let mut output = setup(&mut host, 0, &["documents", "filesystem"]);
    next(&mut output);
    let mut events = local_service(&mut host);
    request(
        &mut host,
        0,
        1,
        api::Request::BufferRead {
            buffer: "unknown".into(),
            expected_revision: "r:0".into(),
            from: 0,
            to: 0,
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
        api::Request::BufferOpen {
            path: "../outside.txt".into(),
            invocation: None,
        },
    );
    assert_eq!(
        complete(&mut host, &mut events, &mut output)
            .await
            .unwrap_err()
            .code,
        api::ErrorCode::InvalidArgument
    );
    let file = std::fs::File::create(root.path().join("huge.txt")).unwrap();
    file.set_len(crate::plugin::filesystem::MAX_DOCUMENT_BYTES as u64 + 1)
        .unwrap();
    drop(file);
    request(
        &mut host,
        0,
        3,
        api::Request::BufferOpen {
            path: "huge.txt".into(),
            invocation: None,
        },
    );
    assert_eq!(
        complete(&mut host, &mut events, &mut output)
            .await
            .unwrap_err()
            .code,
        api::ErrorCode::LimitExceeded
    );
    for serial in 4..=6 {
        request(
            &mut host,
            0,
            serial,
            api::Request::FilesystemList {
                path: ".".into(),
                offset: 0,
                limit: 1,
                expected_revision: None,
            },
        );
    }
    assert_eq!(
        response(&mut output).unwrap_err().code,
        api::ErrorCode::Busy
    );
    complete(&mut host, &mut events, &mut output).await.unwrap();
    complete(&mut host, &mut events, &mut output).await.unwrap();
    assert!(
        host.app.plugins.instances[&0]
            .application
            .local_requests
            .is_empty()
    );
    host.stop_plugin(0, "stopped by user");
}

#[tokio::test]
async fn application_confirmation_preserves_active_and_inactive_directory_edits() {
    let (root, mut host) = host();
    let child = root.path().join("child");
    std::fs::create_dir(&child).unwrap();
    let active = host
        .app
        .host_open_file(root.path().to_path_buf(), true)
        .unwrap();
    let inactive = host.app.host_open_file(child.clone(), false).unwrap();
    assert_eq!(host.app.active().buffer, active);
    assert_ne!(active, inactive);
    let mut texts = Vec::new();
    for index in [active, inactive] {
        let end = host.app.buffers[index].len_chars();
        host.app.buffers[index].apply(&crate::text::Transaction::insert(end, "unsaved.txt\n"));
        texts.push(host.app.buffers[index].to_string());
    }
    let mut output = setup(&mut host, 0, &["filesystem"]);
    next(&mut output);
    let mut events = local_service(&mut host);
    let (directory, revision, _) = list(&mut host, &mut events, &mut output, 1).await;
    request(
        &mut host,
        0,
        2,
        api::Request::FilesystemPrepare {
            directory,
            expected_revision: revision,
            intent: Intent::CreateFile {
                destination: "child/created.txt".into(),
            },
        },
    );
    let api::ResultValue::FilesystemPlan { plan, .. } =
        complete(&mut host, &mut events, &mut output).await.unwrap()
    else {
        panic!()
    };
    let invocation = foreground(&mut host, &mut output);
    request(
        &mut host,
        0,
        3,
        api::Request::FilesystemApply { plan, invocation },
    );
    response(&mut output).unwrap();
    host.app
        .handle_key(KeyStroke::parse("Enter").unwrap())
        .unwrap();
    assert!(child.join("created.txt").is_file());
    for (index, text) in [active, inactive].into_iter().zip(texts) {
        assert!(host.app.buffers[index].dirty);
        assert_eq!(host.app.buffers[index].to_string(), text);
    }
}

#[cfg(unix)]
#[test]
fn local_plans_resolve_static_directory_and_destination_aliases_consistently() {
    use crate::plugin::filesystem::{Prepared, Task};
    let (root, _host) = host();
    let workspace = root.path().join("workspace");
    std::fs::create_dir_all(workspace.join("deep/child")).unwrap();
    std::os::unix::fs::symlink(&workspace, workspace.join("alias")).unwrap();
    std::os::unix::fs::symlink(workspace.join("deep/child"), workspace.join("down")).unwrap();
    for (index, (listing, destination, target)) in [
        ("alias", "new.txt", "new.txt"),
        ("down", "other.txt", "other.txt"),
        ("alias", "down/inside.txt", "deep/child/inside.txt"),
        ("down", "alias/deep/also.txt", "deep/also.txt"),
    ]
    .into_iter()
    .enumerate()
    {
        let Prepared::Directory(directory) = Task::List {
            root: workspace.clone(),
            path: listing.into(),
        }
        .run()
        .unwrap() else {
            panic!()
        };
        let Prepared::Plan(plan) = Task::Prepare {
            root: workspace.clone(),
            directory,
            intent: Intent::CreateFile {
                destination: destination.into(),
            },
        }
        .run()
        .unwrap() else {
            panic!()
        };
        assert_eq!(plan.operations().len(), 1, "case {index}");
        plan.apply(crate::fs_plan::DeletionMode::Permanent).unwrap();
        assert!(workspace.join(target).is_file(), "case {index}");
    }
    assert!(!root.path().join("new.txt").exists());
}
