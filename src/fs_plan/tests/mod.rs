// SPDX-License-Identifier: MPL-2.0

use super::*;
use std::{io, sync::atomic::Ordering};

struct TempDir(PathBuf);
impl TempDir {
    fn new() -> Self {
        let id = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
        let root =
            std::env::temp_dir().join(format!("runyte-plan-safety-{}-{id}", std::process::id()));
        fs::create_dir(&root).unwrap();
        Self(root.canonicalize().unwrap())
    }
    fn join(&self, path: impl AsRef<Path>) -> PathBuf {
        self.0.join(path)
    }
}
impl Drop for TempDir {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

struct NoTrash;
impl TrashBackend for NoTrash {
    fn delete(&self, _: &Path) -> Result<()> {
        panic!("test unexpectedly reached native trash boundary")
    }
}

fn rename_plan(root: &Path, names: &[&str]) -> FsPlan {
    for name in names {
        fs::write(root.join(name), format!("original {name}")).unwrap();
    }
    let snapshot = DirectorySnapshot::read(root).unwrap();
    let desired = snapshot
        .entries()
        .iter()
        .map(|entry| DesiredEntry::existing(entry, format!("{}-new", entry.path.display())))
        .collect();
    FsPlan::build(root.to_path_buf(), snapshot, desired).unwrap()
}

fn copy_plan(root: &Path, directory: bool) -> FsPlan {
    if directory {
        fs::create_dir(root.join("source")).unwrap();
        fs::write(root.join("source/child"), "copied bytes").unwrap();
    } else {
        fs::write(root.join("source"), "copied bytes").unwrap();
    }
    let snapshot = DirectorySnapshot::read(root).unwrap();
    let entry = &snapshot.entries()[0];
    let desired = vec![
        DesiredEntry::existing(entry, "source"),
        DesiredEntry::existing(entry, "destination"),
    ];
    FsPlan::build(root.to_path_buf(), snapshot, desired).unwrap()
}

fn apply(
    plan: &FsPlan,
    hook: impl FnMut(IoStep, &Path, &Path) -> io::Result<()> + 'static,
) -> Result<ApplyReport, ApplyError> {
    plan.apply_with_io(
        DeletionMode::Permanent,
        &NoTrash,
        &mut ApplyIo {
            hook: Some(Box::new(hook)),
        },
    )
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn exclusive_rename_preserves_every_destination_entry_kind() {
    for kind in 0..5 {
        let dir = TempDir::new();
        let source = dir.join("source");
        let target = dir.join("target");
        fs::write(&source, "source").unwrap();
        match kind {
            0 => fs::write(&target, "replacement").unwrap(),
            1 => fs::create_dir(&target).unwrap(),
            2 => {
                fs::create_dir(&target).unwrap();
                fs::write(target.join("child"), "child").unwrap();
            }
            3 | 4 => {
                if kind == 3 {
                    fs::write(dir.join("linked"), "linked").unwrap();
                }
                std::os::unix::fs::symlink("linked", &target).unwrap();
            }
            _ => unreachable!(),
        }
        let identity = Identity::read(&target).unwrap();
        assert_eq!(
            platform::rename_noreplace(&source, &target)
                .unwrap_err()
                .kind(),
            io::ErrorKind::AlreadyExists
        );
        identity.check(&target).unwrap();
        assert_eq!(fs::read_to_string(&source).unwrap(), "source");
        if kind == 0 {
            assert_eq!(fs::read_to_string(&target).unwrap(), "replacement");
        }
        if kind == 2 {
            assert_eq!(fs::read_to_string(target.join("child")).unwrap(), "child");
        }
        if kind == 3 {
            assert_eq!(fs::read_to_string(dir.join("linked")).unwrap(), "linked");
        }
    }
    let dir = TempDir::new();
    fs::create_dir(dir.join("source")).unwrap();
    fs::create_dir(dir.join("target")).unwrap();
    assert_eq!(
        platform::rename_noreplace(&dir.join("source"), &dir.join("target"))
            .unwrap_err()
            .kind(),
        io::ErrorKind::AlreadyExists
    );
    fs::remove_dir(dir.join("target")).unwrap();
    platform::rename_noreplace(&dir.join("source"), &dir.join("target")).unwrap();
    assert!(!dir.join("source").exists());
    assert!(dir.join("target").is_dir());
}

#[test]
fn move_publication_collision_restores_original() {
    let dir = TempDir::new();
    let plan = rename_plan(&dir.0, &["a"]);
    let error = apply(&plan, |step, _, target| {
        if step == IoStep::Publish {
            fs::write(target, "concurrent destination")?;
        }
        Ok(())
    })
    .unwrap_err();
    assert!(error.report.applied.is_empty());
    assert!(error.report.recovery.is_empty());
    assert_eq!(fs::read_to_string(dir.join("a")).unwrap(), "original a");
    assert_eq!(
        fs::read_to_string(dir.join("a-new")).unwrap(),
        "concurrent destination"
    );
    assert_eq!(fs::read_dir(&dir.0).unwrap().count(), 2);
}

#[test]
fn rollback_conflict_preserves_both_files_and_restores_other_originals() {
    let dir = TempDir::new();
    let plan = rename_plan(&dir.0, &["a", "b"]);
    let error = apply(&plan, |step, _, target| {
        if step == IoStep::Publish {
            return Err(io::Error::other("injected publication failure"));
        }
        if step == IoStep::Restore && target.ends_with("a") {
            fs::write(target, "new a")?;
        }
        Ok(())
    })
    .unwrap_err();
    assert_eq!(error.report.recovery.len(), 1);
    let recovery = &error.report.recovery[0];
    assert_eq!(recovery.kind, RecoveryKind::Original);
    assert_eq!(recovery.original, dir.join("a"));
    assert_eq!(
        fs::read_to_string(&recovery.retained).unwrap(),
        "original a"
    );
    assert_eq!(fs::read_to_string(dir.join("a")).unwrap(), "new a");
    assert_eq!(fs::read_to_string(dir.join("b")).unwrap(), "original b");
    assert!(
        error
            .to_string()
            .contains(&recovery.retained.display().to_string())
    );
    assert!(error.to_string().contains("injected publication failure"));
    let retained = recovery.retained.clone();
    drop(plan);
    // A new plan and dropping the old error must not sweep recovery artifacts.
    let snapshot = DirectorySnapshot::read(&dir.0).unwrap();
    let desired = snapshot
        .entries()
        .iter()
        .map(|entry| DesiredEntry::existing(entry, entry.path.clone()))
        .collect();
    FsPlan::build(dir.0.clone(), snapshot, desired)
        .unwrap()
        .apply_with_trash(DeletionMode::Permanent, &NoTrash)
        .unwrap();
    drop(error);
    assert_eq!(fs::read_to_string(retained).unwrap(), "original a");
}

#[test]
fn restoration_io_failure_reports_retained_original() {
    let dir = TempDir::new();
    let plan = rename_plan(&dir.0, &["a"]);
    let error = apply(&plan, |step, _, _| match step {
        IoStep::Publish => Err(io::Error::other("publish failed")),
        IoStep::Restore => Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "restore denied",
        )),
        _ => Ok(()),
    })
    .unwrap_err();
    assert!(!dir.join("a").exists());
    assert_eq!(
        fs::read_to_string(&error.report.recovery[0].retained).unwrap(),
        "original a"
    );
    assert!(error.to_string().contains("restore denied"));
    assert!(error.to_string().contains("publish failed"));
}

#[test]
fn missing_or_substituted_staging_is_not_restored_or_deleted() {
    for substitute in [false, true] {
        let dir = TempDir::new();
        let plan = rename_plan(&dir.0, &["a"]);
        let error = apply(&plan, move |step, source, _| {
            if step == IoStep::Publish {
                // Keep the old inode allocated, so the replacement cannot
                // coincidentally reuse it in this deterministic identity test.
                fs::rename(source, source.with_file_name("displaced"))?;
                if substitute {
                    fs::write(source, "substituted")?;
                }
                return Err(io::Error::other("publication interrupted"));
            }
            Ok(())
        })
        .unwrap_err();
        let entry = &error.report.recovery[0];
        assert_eq!(entry.kind, RecoveryKind::Original);
        assert!(!dir.join("a").exists());
        assert_eq!(
            fs::read_to_string(entry.retained.with_file_name("displaced")).unwrap(),
            "original a"
        );
        if substitute {
            assert_eq!(fs::read_to_string(&entry.retained).unwrap(), "substituted");
        }
    }
}

#[test]
fn allocation_collision_retries_without_touching_competing_entry() {
    let dir = TempDir::new();
    let mut collided = None;
    let mut io = ApplyIo {
        hook: Some(Box::new(move |step, _, target| {
            if step == IoStep::Allocate && collided.is_none() {
                fs::write(target, "competing staging name")?;
                collided = Some(target.to_path_buf());
            }
            Ok(())
        })),
    };
    let tree = OwnedTree::allocate(&dir.0, "test", &mut io).unwrap();
    tree.cleanup(&mut io).unwrap();
    let remaining = fs::read_dir(&dir.0)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    assert_eq!(
        fs::read_to_string(remaining).unwrap(),
        "competing staging name"
    );
    let mut io = ApplyIo {
        hook: Some(Box::new(|_, _, target| {
            fs::create_dir(target)?;
            Ok(())
        })),
    };
    assert_eq!(
        OwnedTree::allocate(&dir.0, "full", &mut io)
            .unwrap_err()
            .kind(),
        io::ErrorKind::AlreadyExists
    );
    assert_eq!(fs::read_dir(&dir.0).unwrap().count(), 65);
}

#[test]
fn staged_payload_collision_is_neither_overwritten_nor_cleaned() {
    let dir = TempDir::new();
    let plan = rename_plan(&dir.0, &["a"]);
    let error = apply(&plan, |step, _, target| {
        if step == IoStep::Stage {
            fs::write(target, "competing payload")?;
        }
        Ok(())
    })
    .unwrap_err();
    assert_eq!(fs::read_to_string(dir.join("a")).unwrap(), "original a");
    assert_eq!(error.report.recovery.len(), 1);
    assert_eq!(
        fs::read_to_string(error.report.recovery[0].retained.join("entry")).unwrap(),
        "competing payload"
    );
}

#[test]
fn copy_publication_collision_preserves_source_and_destination() {
    for directory in [false, true] {
        let dir = TempDir::new();
        let plan = copy_plan(&dir.0, directory);
        let error = apply(&plan, |step, _, target| {
            if step == IoStep::Publish {
                fs::write(target, "competing destination")?;
            }
            Ok(())
        })
        .unwrap_err();
        assert!(error.report.recovery.is_empty());
        assert_eq!(
            fs::read_to_string(dir.join(if directory { "source/child" } else { "source" }))
                .unwrap(),
            "copied bytes"
        );
        assert_eq!(
            fs::read_to_string(dir.join("destination")).unwrap(),
            "competing destination"
        );
        assert_eq!(fs::read_dir(&dir.0).unwrap().count(), 2);
    }
}

#[test]
fn copy_creation_collision_is_not_truncated_or_cleaned() {
    let dir = TempDir::new();
    let plan = copy_plan(&dir.0, true);
    let error = apply(&plan, |step, source, target| {
        if step == IoStep::CopyEntry && source.ends_with("child") {
            fs::write(target, "competing copy child")?;
        }
        Ok(())
    })
    .unwrap_err();
    let retained = &error.report.recovery[0].retained;
    assert_eq!(
        fs::read_to_string(retained.join("entry/child")).unwrap(),
        "competing copy child"
    );
    assert_eq!(
        fs::read_to_string(dir.join("source/child")).unwrap(),
        "copied bytes"
    );
    assert!(!dir.join("destination").exists());
}

#[test]
fn cleanup_retains_unexpected_children_and_original_failure() {
    let dir = TempDir::new();
    let plan = copy_plan(&dir.0, true);
    let error = apply(&plan, |step, source, _| {
        if step == IoStep::Publish {
            return Err(io::Error::other("publication denied"));
        }
        if step == IoStep::Cleanup
            && source
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with(".runyte-copy-")
        {
            fs::write(source.join("unexpected"), "concurrent child")?;
        }
        Ok(())
    })
    .unwrap_err();
    assert!(error.to_string().contains("publication denied"));
    let retained = &error.report.recovery[0].retained;
    assert_eq!(
        fs::read_to_string(retained.join("unexpected")).unwrap(),
        "concurrent child"
    );
    assert_eq!(
        fs::read_to_string(retained.join("entry/child")).unwrap(),
        "copied bytes"
    );
}

#[test]
fn unsupported_rename_is_detected_before_mixed_plan_deletes() {
    let dir = TempDir::new();
    let mut plan = rename_plan(&dir.0, &["a", "b"]);
    plan.operations
        .retain(|op| op.staged_source() != Some(Path::new("b")));
    plan.operations.push(FsOperation::Delete {
        path: "b".into(),
        kind: EntryKind::File,
    });
    let error = apply(&plan, |step, _, _| {
        if step == IoStep::Probe {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "exclusive rename unsupported",
            ));
        }
        Ok(())
    })
    .unwrap_err();
    assert!(error.report.applied.is_empty());
    assert!(error.report.recovery.is_empty());
    assert_eq!(fs::read_to_string(dir.join("a")).unwrap(), "original a");
    assert_eq!(fs::read_to_string(dir.join("b")).unwrap(), "original b");
    assert!(error.to_string().contains("exclusive rename unsupported"));
    assert_eq!(fs::read_dir(&dir.0).unwrap().count(), 2);
}

#[test]
fn invalid_target_is_rejected_before_any_operation() {
    let dir = TempDir::new();
    let snapshot = DirectorySnapshot::read(&dir.0).unwrap();
    let plan = FsPlan::build(
        dir.0.clone(),
        snapshot,
        vec![
            DesiredEntry::create("a", EntryKind::File),
            DesiredEntry::create("z".repeat(300), EntryKind::File),
        ],
    )
    .unwrap();
    let error = apply(&plan, |_, _, _| Ok(())).unwrap_err();
    assert!(error.report.applied.is_empty());
    assert_eq!(fs::read_dir(&dir.0).unwrap().count(), 0);
}

#[test]
fn publication_success_with_cleanup_failure_reports_applied_operation() {
    let dir = TempDir::new();
    let plan = rename_plan(&dir.0, &["a"]);
    let error = apply(&plan, |step, source, _| {
        if step == IoStep::Cleanup
            && source
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with(".runyte-move-")
        {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "cleanup denied",
            ));
        }
        Ok(())
    })
    .unwrap_err();
    assert_eq!(error.report.applied.len(), 1);
    assert_eq!(error.report.recovery[0].kind, RecoveryKind::Staging);
    assert!(!dir.join("a").exists());
    assert_eq!(fs::read_to_string(dir.join("a-new")).unwrap(), "original a");
    assert!(error.to_string().contains("cleanup denied"));
}

#[cfg(unix)]
#[test]
fn exclusive_rename_rejects_nul_without_touching_source() {
    use std::os::unix::ffi::OsStrExt;
    let dir = TempDir::new();
    fs::write(dir.join("source"), "source").unwrap();
    let bad = Path::new(std::ffi::OsStr::from_bytes(b"bad\0path"));
    assert_eq!(
        platform::rename_noreplace(&dir.join("source"), bad)
            .unwrap_err()
            .kind(),
        io::ErrorKind::InvalidInput
    );
    assert_eq!(fs::read_to_string(dir.join("source")).unwrap(), "source");
}

#[test]
fn partial_copy_failure_cleans_only_owned_entries() {
    let dir = TempDir::new();
    let plan = copy_plan(&dir.0, true);
    let error = apply(&plan, |step, source, _| {
        if step == IoStep::CopyEntry && source.ends_with("child") {
            return Err(io::Error::other("injected read failure"));
        }
        Ok(())
    })
    .unwrap_err();
    assert!(error.to_string().contains("injected read failure"));
    assert!(error.report.recovery.is_empty());
    assert_eq!(fs::read_dir(&dir.0).unwrap().count(), 1);
    assert_eq!(
        fs::read_to_string(dir.join("source/child")).unwrap(),
        "copied bytes"
    );
}

#[test]
fn copy_cleanup_preserves_a_replacement_symlink_and_its_target() {
    #[cfg(unix)]
    {
        let dir = TempDir::new();
        let plan = copy_plan(&dir.0, false);
        let outside = dir.join("outside");
        // Create after planning, but hidden from the directory snapshot, by
        // injecting only once the copy has reached publication.
        let linked = outside.clone();
        let error = apply(&plan, move |step, source, _| {
            if step == IoStep::Publish {
                fs::write(&linked, "external bytes")?;
                fs::rename(source, source.with_file_name("displaced"))?;
                std::os::unix::fs::symlink(&linked, source)?;
                return Err(io::Error::other("publication interrupted"));
            }
            Ok(())
        })
        .unwrap_err();
        assert_eq!(fs::read_to_string(outside).unwrap(), "external bytes");
        let retained = &error.report.recovery[0].retained;
        assert!(
            fs::symlink_metadata(retained.join("entry"))
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(
            fs::read_to_string(retained.join("displaced")).unwrap(),
            "copied bytes"
        );
    }
}

#[test]
fn missing_source_parent_during_restore_retains_original() {
    let dir = TempDir::new();
    fs::create_dir(dir.join("source-dir")).unwrap();
    let plan = rename_plan(&dir.join("source-dir"), &["a"]);
    // Move the root itself only after staging. This is outside confinement,
    // but the missing staged path must be reported as uncertain, not success.
    let root = dir.join("source-dir");
    let displaced = dir.join("displaced");
    let error = apply(&plan, move |step, _, _| {
        if step == IoStep::Publish {
            fs::rename(&root, &displaced)?;
            return Err(io::Error::other("parent moved"));
        }
        Ok(())
    })
    .unwrap_err();
    assert_eq!(error.report.recovery[0].kind, RecoveryKind::Original);
    assert!(error.report.recovery[0].reason.contains("No such file"));
    let retained = error.report.recovery[0]
        .retained
        .strip_prefix(dir.join("source-dir"))
        .unwrap();
    assert_eq!(
        fs::read_to_string(dir.join("displaced").join(retained)).unwrap(),
        "original a"
    );
}

#[test]
fn later_staging_failure_restores_earlier_sources() {
    let dir = TempDir::new();
    let plan = rename_plan(&dir.0, &["a", "b"]);
    let error = apply(&plan, |step, source, _| {
        if step == IoStep::Stage && source.ends_with("b") {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "staging denied",
            ));
        }
        Ok(())
    })
    .unwrap_err();
    assert!(error.report.recovery.is_empty());
    assert!(error.report.applied.is_empty());
    assert_eq!(fs::read_to_string(dir.join("a")).unwrap(), "original a");
    assert_eq!(fs::read_to_string(dir.join("b")).unwrap(), "original b");
    assert_eq!(fs::read_dir(&dir.0).unwrap().count(), 2);
}

#[test]
fn late_cross_device_failure_never_falls_back_to_copy_delete() {
    let dir = TempDir::new();
    let plan = rename_plan(&dir.0, &["a"]);
    let error = apply(&plan, |step, _, _| {
        if step == IoStep::Publish {
            return Err(io::Error::from_raw_os_error(libc::EXDEV));
        }
        Ok(())
    })
    .unwrap_err();
    assert!(error.report.recovery.is_empty());
    assert_eq!(fs::read_to_string(dir.join("a")).unwrap(), "original a");
    assert!(!dir.join("a-new").exists());
}

#[test]
fn mixed_plan_reports_deletion_before_later_collision_in_both_modes() {
    struct FakeTrash;
    impl TrashBackend for FakeTrash {
        fn delete(&self, path: &Path) -> Result<()> {
            fs::remove_file(path)?;
            Ok(())
        }
    }
    for mode in [DeletionMode::Permanent, DeletionMode::Trash] {
        let dir = TempDir::new();
        let mut plan = rename_plan(&dir.0, &["a", "b"]);
        plan.operations
            .retain(|op| op.staged_source() != Some(Path::new("b")));
        let deletion = FsOperation::Delete {
            path: "b".into(),
            kind: EntryKind::File,
        };
        plan.operations.push(deletion.clone());
        let mut io = ApplyIo {
            hook: Some(Box::new(|step, _, target| {
                if step == IoStep::Publish {
                    fs::write(target, "concurrent destination")?;
                }
                Ok(())
            })),
        };
        let error = plan.apply_with_io(mode, &FakeTrash, &mut io).unwrap_err();
        assert_eq!(error.report.applied, vec![deletion]);
        assert!(!dir.join("b").exists());
        assert_eq!(fs::read_to_string(dir.join("a")).unwrap(), "original a");
        assert_eq!(
            fs::read_to_string(dir.join("a-new")).unwrap(),
            "concurrent destination"
        );
    }
}
