// SPDX-License-Identifier: MPL-2.0

use super::filesystem::{
    applied, applied_event, complete, finished, foreground, list, local_service, response, started,
};
use super::*;
use crate::input::KeyStroke;
use crate::plugin::filesystem::Intent;

async fn prepare_tree(
    host: &mut WorkspaceHost,
    events: &mut mpsc::Receiver<Event>,
    output: &mut mpsc::Receiver<HostMessage>,
    operation: &str,
) -> Result<String, api::Error> {
    let (directory, revision, entries) = list(host, events, output, 1).await;
    let entry = entries
        .iter()
        .find(|entry| entry.name == "資料")
        .unwrap()
        .entry
        .clone();
    let intent = match operation {
        "copy" => Intent::Copy {
            entry,
            destination: "複製".into(),
        },
        "rename" => Intent::Rename {
            entry,
            destination: "改名".into(),
        },
        _ => Intent::Trash { entry },
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
    match complete(host, events, output).await? {
        api::ResultValue::FilesystemPlan { plan, .. } => Ok(plan),
        _ => panic!(),
    }
}
fn show(host: &mut WorkspaceHost, output: &mut mpsc::Receiver<HostMessage>, plan: String) {
    let invocation = foreground(host, output);
    request(
        host,
        0,
        3,
        api::Request::FilesystemApply { plan, invocation },
    );
    response(output).unwrap();
}
fn accept(host: &mut WorkspaceHost, output: &mut mpsc::Receiver<HostMessage>, permanent: bool) {
    host.app
        .handle_key(if permanent {
            KeyStroke::char('P')
        } else {
            KeyStroke::parse("Enter").unwrap()
        })
        .unwrap();
    host.sync_plugin_observers();
    started(output);
}
struct TemporaryTrash(std::path::PathBuf);
impl crate::fs_plan::TrashBackend for TemporaryTrash {
    fn delete(&self, path: &std::path::Path) -> anyhow::Result<()> {
        std::fs::rename(path, &self.0)?;
        Ok(())
    }
}

#[tokio::test]
async fn recursive_directory_operations_require_confirmation_and_preserve_unicode_and_symlinks() {
    for operation in ["copy", "rename", "trash", "permanent", "cancel"] {
        let (root, mut host) = host();
        let source = root.path().join("資料");
        std::fs::create_dir_all(source.join("子/é")).unwrap();
        std::fs::write(source.join("子/é/猫.txt"), "Unicode é猫\n").unwrap();
        let outside = root.path().join("outside.txt");
        std::fs::write(&outside, "untouched outside\n").unwrap();
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&outside, source.join("子/link")).unwrap();
            std::os::unix::fs::symlink("missing", source.join("broken")).unwrap();
        }
        let trashed = root.path().join("retained-trash");
        host.app
            .set_trash_backend(Box::new(TemporaryTrash(trashed.clone())));
        let mut output = setup(&mut host, 0, &["filesystem", "jobs"]);
        next(&mut output);
        let mut events = local_service(&mut host);
        let plan = prepare_tree(&mut host, &mut events, &mut output, operation)
            .await
            .unwrap();
        assert!(source.join("子/é/猫.txt").exists());
        assert!(!root.path().join("複製").exists());
        show(&mut host, &mut output, plan);
        assert!(source.join("子/é/猫.txt").exists());
        assert!(!trashed.exists());
        if operation == "cancel" {
            host.app
                .handle_key(KeyStroke::parse("Esc").unwrap())
                .unwrap();
            host.sync_plugin_observers();
            assert!(
                matches!(next(&mut output), api::HostMessage::Event { data: api::EventData::FilesystemFinished(value), .. } if value.state == "cancelled")
            );
            assert!(source.join("子/é/猫.txt").exists());
            continue;
        }
        accept(&mut host, &mut output, operation == "permanent");
        assert_eq!(
            applied(&mut host, &mut events, &mut output).await.state,
            "succeeded",
            "{operation}"
        );
        let retained = match operation {
            "copy" => {
                assert!(source.exists());
                Some(root.path().join("複製"))
            }
            "rename" => {
                assert!(!source.exists());
                Some(root.path().join("改名"))
            }
            "trash" => {
                assert!(!source.exists());
                Some(trashed)
            }
            _ => {
                assert!(!source.exists());
                None
            }
        };
        if let Some(retained) = retained {
            assert_eq!(
                std::fs::read_to_string(retained.join("子/é/猫.txt")).unwrap(),
                "Unicode é猫\n"
            );
            #[cfg(unix)]
            {
                assert!(
                    std::fs::symlink_metadata(retained.join("子/link"))
                        .unwrap()
                        .file_type()
                        .is_symlink()
                );
                assert_eq!(
                    std::fs::read_link(retained.join("子/link")).unwrap(),
                    outside
                );
                assert_eq!(
                    std::fs::read_link(retained.join("broken")).unwrap(),
                    std::path::PathBuf::from("missing")
                );
            }
        }
        assert_eq!(
            std::fs::read_to_string(outside).unwrap(),
            "untouched outside\n"
        );
    }
}

#[tokio::test]
async fn recursive_rename_retargets_dirty_open_descendants_and_keeps_undo() {
    let (root, mut host) = host();
    let source = root.path().join("資料");
    std::fs::create_dir_all(source.join("子")).unwrap();
    let file = source.join("子/猫.txt");
    std::fs::write(&file, "baseline é\n").unwrap();
    let buffer = host.app.host_open_file(file, true).unwrap();
    host.apply_expected_transaction(
        BufferId::from_index(buffer),
        BufferRevision::from_raw(host.app.buffers[buffer].revision()),
        Transaction::insert(0, "local "),
    )
    .unwrap();
    host.app.buffers[buffer].commit_undo_group();
    let mut output = setup(&mut host, 0, &["filesystem", "jobs"]);
    next(&mut output);
    let mut events = local_service(&mut host);
    let plan = prepare_tree(&mut host, &mut events, &mut output, "rename")
        .await
        .unwrap();
    show(&mut host, &mut output, plan);
    accept(&mut host, &mut output, false);
    let event = applied_event(&mut events).await;
    host.handle_plugin_event(event);
    assert_eq!(finished(&mut output).state, "succeeded");
    assert_eq!(
        host.app.buffers[buffer].path.as_ref(),
        Some(&root.path().join("改名/子/猫.txt"))
    );
    assert_eq!(host.app.buffers[buffer].to_string(), "local baseline é\n");
    assert!(host.app.buffers[buffer].dirty);
    host.app.handle_key(KeyStroke::char('u')).unwrap();
    assert_eq!(host.app.buffers[buffer].to_string(), "baseline é\n");
    assert!(!host.app.buffers[buffer].dirty);
    assert_eq!(
        std::fs::read_to_string(root.path().join("改名/子/猫.txt")).unwrap(),
        "baseline é\n"
    );
}

fn exceed(source: &std::path::Path, limit: &str) {
    match limit {
        "entries" => {
            for n in 0..1025 {
                std::fs::write(source.join(format!("entry-{n}")), "").unwrap();
            }
        }
        "depth" => {
            let mut path = source.to_path_buf();
            for _ in 0..33 {
                path.push("深");
            }
            std::fs::create_dir_all(path).unwrap();
        }
        "bytes" => {
            let file = std::fs::File::create(source.join("large.bin")).unwrap();
            file.set_len(64 * 1024 * 1024 + 1).unwrap();
        }
        _ => {
            std::fs::write(source.join("子/kept.txt"), "changed descendant\n").unwrap();
        }
    }
}

#[tokio::test]
async fn recursive_prepare_rejects_entry_depth_and_data_limits_without_mutation() {
    for limit in ["entries", "depth", "bytes"] {
        let (root, mut host) = host();
        let source = root.path().join("資料");
        std::fs::create_dir(&source).unwrap();
        exceed(&source, limit);
        let mut output = setup(&mut host, 0, &["filesystem", "jobs"]);
        next(&mut output);
        let mut events = local_service(&mut host);
        assert_eq!(
            prepare_tree(&mut host, &mut events, &mut output, "copy")
                .await
                .unwrap_err()
                .code,
            api::ErrorCode::LimitExceeded,
            "{limit}"
        );
        assert!(source.is_dir());
        assert!(!root.path().join("複製").exists());
        assert!(host.app.fs_confirmation.is_none());
        assert!(host.app.plugins.instances[&0].application.plans.is_empty());
        assert_eq!(
            host.app.plugins.instances[&0].application.retained_payload,
            crate::plugin::filesystem::DIRECTORY_CHARGE
        );
    }
}

#[tokio::test]
async fn recursive_apply_revalidates_changed_and_grown_descendants_before_mutating() {
    for limit in ["entries", "depth", "bytes", "changed"] {
        let (root, mut host) = host();
        let source = root.path().join("資料");
        std::fs::create_dir_all(source.join("子")).unwrap();
        std::fs::write(source.join("子/kept.txt"), "original descendant\n").unwrap();
        let mut output = setup(&mut host, 0, &["filesystem", "jobs"]);
        next(&mut output);
        let mut events = local_service(&mut host);
        let plan = prepare_tree(&mut host, &mut events, &mut output, "copy")
            .await
            .unwrap();
        show(&mut host, &mut output, plan);
        let root_before =
            crate::fs_plan::SourceFingerprint::shallow(&source, "test source").unwrap();
        if limit == "changed" {
            exceed(&source, limit);
        } else {
            exceed(&source.join("子"), limit);
        }
        assert_eq!(
            crate::fs_plan::SourceFingerprint::shallow(&source, "test source").unwrap(),
            root_before
        );
        accept(&mut host, &mut output, false);
        let result = applied(&mut host, &mut events, &mut output).await;
        assert_eq!(result.state, "failed", "{limit}");
        assert_eq!(result.applied, 0);
        assert!(!result.recovery);
        assert!(source.is_dir());
        assert!(!root.path().join("複製").exists());
        assert_eq!(
            std::fs::read_to_string(source.join("子/kept.txt")).unwrap(),
            if limit == "changed" {
                "changed descendant\n"
            } else {
                "original descendant\n"
            }
        );
    }
}
