// SPDX-License-Identifier: MPL-2.0

use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use runyte::fs_plan::{
    DeletionMode, DesiredEntry, DirectorySnapshot, EntryKind, FsOperation, FsPlan,
    SourceFingerprint, TransferMode,
};

static NEXT_DIR: AtomicU64 = AtomicU64::new(1);

struct TempDir(PathBuf);

impl TempDir {
    fn new(label: &str) -> Self {
        let number = NEXT_DIR.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "runyte-fs-plan-{label}-{}-{number}",
            std::process::id()
        ));
        fs::create_dir(&path).unwrap();
        Self(path.canonicalize().unwrap())
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn reordering_entries_produces_no_operation() {
    let directory = TempDir::new("reorder");
    fs::write(directory.path().join("a"), "a").unwrap();
    fs::write(directory.path().join("b"), "b").unwrap();
    let snapshot = DirectorySnapshot::read(directory.path()).unwrap();
    let desired = snapshot
        .entries()
        .iter()
        .rev()
        .map(|entry| DesiredEntry::existing(entry, entry.path.clone()))
        .collect();

    let plan = FsPlan::build(directory.path().to_path_buf(), snapshot, desired).unwrap();

    assert!(plan.is_empty());
}

#[test]
fn a_rename_cycle_uses_temporaries_without_losing_either_file() {
    let directory = TempDir::new("cycle");
    fs::write(directory.path().join("a"), "from a").unwrap();
    fs::write(directory.path().join("b"), "from b").unwrap();
    let snapshot = DirectorySnapshot::read(directory.path()).unwrap();
    let a = snapshot
        .entries()
        .iter()
        .find(|entry| entry.path == Path::new("a"))
        .unwrap();
    let b = snapshot
        .entries()
        .iter()
        .find(|entry| entry.path == Path::new("b"))
        .unwrap();
    let plan = FsPlan::build(
        directory.path().to_path_buf(),
        snapshot.clone(),
        vec![
            DesiredEntry::existing(a, "b"),
            DesiredEntry::existing(b, "a"),
        ],
    )
    .unwrap();

    let report = plan.apply(DeletionMode::Permanent).unwrap();

    assert_eq!(report.applied.len(), 2);
    assert_eq!(
        fs::read_to_string(directory.path().join("a")).unwrap(),
        "from b"
    );
    assert_eq!(
        fs::read_to_string(directory.path().join("b")).unwrap(),
        "from a"
    );
    assert!(fs::read_dir(directory.path()).unwrap().all(|entry| {
        !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".runyte-move-")
    }));
}

#[test]
fn created_directories_are_ordered_before_moves_into_them() {
    let directory = TempDir::new("move");
    fs::write(directory.path().join("source"), "content").unwrap();
    let snapshot = DirectorySnapshot::read(directory.path()).unwrap();
    let source = snapshot.entries().first().unwrap();
    let plan = FsPlan::build(
        directory.path().to_path_buf(),
        snapshot.clone(),
        vec![
            DesiredEntry::create("nested", EntryKind::Directory),
            DesiredEntry::existing(source, "nested/source"),
        ],
    )
    .unwrap();
    assert!(matches!(
        plan.operations(),
        [
            FsOperation::Create {
                path: nested,
                kind: EntryKind::Directory,
            },
            FsOperation::Move { from, to, .. },
        ] if nested == Path::new("nested")
            && from == Path::new("source")
            && to == Path::new("nested/source")
    ));

    plan.apply(DeletionMode::Permanent).unwrap();

    assert_eq!(
        fs::read_to_string(directory.path().join("nested/source")).unwrap(),
        "content"
    );
}

#[test]
fn a_move_cannot_target_a_directory_that_the_same_plan_moves_away() {
    let directory = TempDir::new("moving-parent");
    fs::create_dir(directory.path().join("directory")).unwrap();
    fs::write(directory.path().join("source"), "content").unwrap();
    let snapshot = DirectorySnapshot::read(directory.path()).unwrap();
    let child_directory = snapshot
        .entries()
        .iter()
        .find(|entry| entry.path == Path::new("directory"))
        .unwrap();
    let source = snapshot
        .entries()
        .iter()
        .find(|entry| entry.path == Path::new("source"))
        .unwrap();
    let plan = FsPlan::build(
        directory.path().to_path_buf(),
        snapshot.clone(),
        vec![
            DesiredEntry::existing(child_directory, "renamed"),
            DesiredEntry::existing(source, "directory/source"),
        ],
    )
    .unwrap();

    let error = plan.apply(DeletionMode::Permanent).unwrap_err();

    assert!(error.to_string().contains("will be moved or deleted"));
    assert!(directory.path().join("directory").is_dir());
    assert!(directory.path().join("source").is_file());
    assert!(!directory.path().join("renamed").exists());
}

#[test]
fn a_moved_directory_can_receive_a_later_move() {
    let directory = TempDir::new("moved-parent");
    fs::create_dir(directory.path().join("directory")).unwrap();
    fs::write(directory.path().join("source"), "content").unwrap();
    let snapshot = DirectorySnapshot::read(directory.path()).unwrap();
    let child_directory = snapshot
        .entries()
        .iter()
        .find(|entry| entry.path == Path::new("directory"))
        .unwrap();
    let source = snapshot
        .entries()
        .iter()
        .find(|entry| entry.path == Path::new("source"))
        .unwrap();
    let plan = FsPlan::build(
        directory.path().to_path_buf(),
        snapshot.clone(),
        vec![
            DesiredEntry::existing(child_directory, "renamed"),
            DesiredEntry::existing(source, "renamed/source"),
        ],
    )
    .unwrap();

    plan.apply(DeletionMode::Permanent).unwrap();

    assert_eq!(
        fs::read_to_string(directory.path().join("renamed/source")).unwrap(),
        "content"
    );
}

#[test]
fn a_new_entry_can_be_created_inside_a_directory_the_same_plan_renames() {
    let directory = TempDir::new("create-inside-renamed");
    fs::create_dir(directory.path().join("a")).unwrap();
    fs::write(directory.path().join("a/kept"), "content").unwrap();
    let snapshot = DirectorySnapshot::read(directory.path()).unwrap();
    let renamed = snapshot
        .entries()
        .iter()
        .find(|entry| entry.path == Path::new("a"))
        .unwrap();
    let plan = FsPlan::build(
        directory.path().to_path_buf(),
        snapshot.clone(),
        vec![
            DesiredEntry::existing(renamed, "b"),
            DesiredEntry::create("b/added", EntryKind::File),
        ],
    )
    .unwrap();

    plan.apply(DeletionMode::Permanent).unwrap();

    assert!(!directory.path().join("a").exists());
    assert_eq!(
        fs::read_to_string(directory.path().join("b/kept")).unwrap(),
        "content"
    );
    assert!(directory.path().join("b/added").is_file());
}

#[test]
fn one_plan_can_mix_multiple_creates_a_rename_and_multiple_moves() {
    let directory = TempDir::new("mixed-operations");
    fs::write(directory.path().join("rename-me"), "renamed contents").unwrap();
    fs::write(directory.path().join("move-a"), "a contents").unwrap();
    fs::write(directory.path().join("move-b"), "b contents").unwrap();
    let snapshot = DirectorySnapshot::read(directory.path()).unwrap();
    let entry = |name: &str| {
        snapshot
            .entries()
            .iter()
            .find(|entry| entry.path == Path::new(name))
            .unwrap()
    };
    let plan = FsPlan::build(
        directory.path().to_path_buf(),
        snapshot.clone(),
        vec![
            DesiredEntry::create("created-a", EntryKind::File),
            DesiredEntry::create("created-b", EntryKind::File),
            DesiredEntry::create("folder", EntryKind::Directory),
            DesiredEntry::existing(entry("rename-me"), "renamed"),
            DesiredEntry::existing(entry("move-a"), "folder/move-a"),
            DesiredEntry::existing(entry("move-b"), "folder/move-b"),
        ],
    )
    .unwrap();

    let report = plan.apply(DeletionMode::Permanent).unwrap();

    assert_eq!(report.applied.len(), 6);
    assert!(directory.path().join("created-a").is_file());
    assert!(directory.path().join("created-b").is_file());
    assert_eq!(
        fs::read_to_string(directory.path().join("renamed")).unwrap(),
        "renamed contents"
    );
    assert_eq!(
        fs::read_to_string(directory.path().join("folder/move-a")).unwrap(),
        "a contents"
    );
    assert_eq!(
        fs::read_to_string(directory.path().join("folder/move-b")).unwrap(),
        "b contents"
    );
}

#[test]
fn a_changed_directory_conflicts_before_any_operation() {
    let directory = TempDir::new("conflict");
    let snapshot = DirectorySnapshot::read(directory.path()).unwrap();
    let plan = FsPlan::build(
        directory.path().to_path_buf(),
        snapshot,
        vec![DesiredEntry::create("planned", EntryKind::File)],
    )
    .unwrap();
    fs::write(directory.path().join("external"), "changed").unwrap();

    let error = plan.apply(DeletionMode::Permanent).unwrap_err();

    assert!(error.to_string().contains("directory changed on disk"));
    assert!(error.report.applied.is_empty());
    assert!(!directory.path().join("planned").exists());
}

#[test]
fn changes_inside_an_unaffected_child_directory_do_not_stale_the_plan() {
    let directory = TempDir::new("unaffected-child-change");
    let active = directory.path().join("active");
    let removed = directory.path().join("removed");
    fs::create_dir(&active).unwrap();
    fs::create_dir(&removed).unwrap();
    let snapshot = DirectorySnapshot::read(directory.path()).unwrap();
    let active_entry = snapshot
        .entries()
        .iter()
        .find(|entry| entry.path == Path::new("active"))
        .unwrap();
    let plan = FsPlan::build(
        directory.path().to_path_buf(),
        snapshot.clone(),
        vec![DesiredEntry::existing(active_entry, "active")],
    )
    .unwrap();
    fs::write(active.join("runtime-state"), "changed").unwrap();

    plan.apply(DeletionMode::Permanent).unwrap();

    assert!(active.join("runtime-state").is_file());
    assert!(!removed.exists());
}

#[test]
fn changes_to_an_unaffected_file_do_not_stale_the_plan() {
    let directory = TempDir::new("unaffected-file-change");
    fs::write(directory.path().join("kept"), "before").unwrap();
    let snapshot = DirectorySnapshot::read(directory.path()).unwrap();
    let kept = snapshot.entries().first().unwrap();
    let plan = FsPlan::build(
        directory.path().to_path_buf(),
        snapshot.clone(),
        vec![
            DesiredEntry::existing(kept, "kept"),
            DesiredEntry::create("created", EntryKind::File),
        ],
    )
    .unwrap();
    fs::write(directory.path().join("kept"), "changed contents").unwrap();

    plan.apply(DeletionMode::Permanent).unwrap();

    assert_eq!(
        fs::read_to_string(directory.path().join("kept")).unwrap(),
        "changed contents"
    );
    assert!(directory.path().join("created").is_file());
}

#[test]
fn a_changed_file_still_stales_a_plan_that_moves_it() {
    let directory = TempDir::new("changed-move-source");
    fs::write(directory.path().join("before"), "original").unwrap();
    let snapshot = DirectorySnapshot::read(directory.path()).unwrap();
    let before = snapshot.entries().first().unwrap();
    let plan = FsPlan::build(
        directory.path().to_path_buf(),
        snapshot.clone(),
        vec![DesiredEntry::existing(before, "after")],
    )
    .unwrap();
    fs::write(directory.path().join("before"), "changed contents").unwrap();

    let error = plan.apply(DeletionMode::Permanent).unwrap_err();

    assert!(error.to_string().contains("directory changed on disk"));
    assert!(directory.path().join("before").is_file());
    assert!(!directory.path().join("after").exists());
}

#[test]
fn changes_inside_a_child_directory_being_deleted_stale_the_plan() {
    let directory = TempDir::new("deleted-child-change");
    let removed = directory.path().join("removed");
    fs::create_dir(&removed).unwrap();
    let snapshot = DirectorySnapshot::read(directory.path()).unwrap();
    let plan = FsPlan::build(directory.path().to_path_buf(), snapshot, Vec::new()).unwrap();
    fs::write(removed.join("new-content"), "changed").unwrap();

    let error = plan.apply(DeletionMode::Permanent).unwrap_err();

    assert!(error.to_string().contains("directory changed on disk"));
    assert!(error.report.applied.is_empty());
    assert!(removed.join("new-content").is_file());
}

#[test]
fn changes_below_a_nested_directory_stale_a_confirmed_recursive_delete() {
    let directory = TempDir::new("nested-deleted-child-change");
    let removed = directory.path().join("removed");
    let nested = removed.join("nested");
    fs::create_dir_all(&nested).unwrap();
    fs::write(nested.join("kept"), "original").unwrap();
    let snapshot = DirectorySnapshot::read(directory.path()).unwrap();
    let plan = FsPlan::build(directory.path().to_path_buf(), snapshot, Vec::new()).unwrap();

    // Changing a grandchild does not change `removed`'s own metadata. The
    // source tree captured with the confirmation still has to notice it.
    fs::write(nested.join("added-after-confirmation"), "changed").unwrap();

    let error = plan.apply(DeletionMode::Permanent).unwrap_err();

    assert!(error.to_string().contains("directory changed on disk"));
    assert!(error.report.applied.is_empty());
    assert!(nested.join("added-after-confirmation").is_file());
}

#[test]
fn a_partial_failure_reports_exactly_what_was_applied() {
    let directory = TempDir::new("partial");
    let snapshot = DirectorySnapshot::read(directory.path()).unwrap();
    let plan = FsPlan::build(
        directory.path().to_path_buf(),
        snapshot,
        vec![
            DesiredEntry::create("a-created", EntryKind::File),
            DesiredEntry::create("z".repeat(300), EntryKind::File),
        ],
    )
    .unwrap();

    let error = plan.apply(DeletionMode::Permanent).unwrap_err();

    assert_eq!(
        error.report.applied,
        vec![FsOperation::Create {
            path: PathBuf::from("a-created"),
            kind: EntryKind::File,
        }]
    );
    assert!(
        error
            .to_string()
            .contains(&format!("create {}", "z".repeat(300)))
    );
    assert!(error.to_string().contains("applied: create a-created"));
    assert!(directory.path().join("a-created").is_file());
    assert!(!directory.path().join("z".repeat(300)).exists());
}

#[cfg(unix)]
#[test]
fn a_dangling_symlink_is_restored_when_a_later_plan_step_fails() {
    use std::os::unix::fs::symlink;

    let directory = TempDir::new("dangling-symlink-rollback");
    let link = directory.path().join("link");
    symlink("missing-target", &link).unwrap();
    let snapshot = DirectorySnapshot::read(directory.path()).unwrap();
    let entry = snapshot.entries().first().unwrap();
    let plan = FsPlan::build(
        directory.path().to_path_buf(),
        snapshot.clone(),
        vec![
            DesiredEntry::existing(entry, "renamed-link"),
            DesiredEntry::create("z".repeat(300), EntryKind::File),
        ],
    )
    .unwrap();

    let error = plan.apply(DeletionMode::Permanent).unwrap_err();

    assert!(error.report.applied.is_empty());
    assert_eq!(fs::read_link(&link).unwrap(), Path::new("missing-target"));
    assert!(!directory.path().join("renamed-link").exists());
    assert!(fs::read_dir(directory.path()).unwrap().all(|entry| {
        !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".runyte-move-")
    }));
}

#[test]
fn directory_operations_are_marked_as_recursive_in_the_review_text() {
    let directory = TempDir::new("directory-review-marker");
    fs::create_dir(directory.path().join("removed")).unwrap();
    let snapshot = DirectorySnapshot::read(directory.path()).unwrap();
    let plan = FsPlan::build(directory.path().to_path_buf(), snapshot, Vec::new()).unwrap();

    assert_eq!(plan.lines(), vec!["delete removed/"]);
}

#[test]
fn parent_relative_paths_can_create_outside_the_explorer_root() {
    let parent = TempDir::new("outside-create");
    let source = parent.path().join("source");
    let destination = parent.path().join("destination");
    fs::create_dir(&source).unwrap();
    fs::create_dir(&destination).unwrap();
    let snapshot = DirectorySnapshot::read(&source).unwrap();
    let plan = FsPlan::build(
        source,
        snapshot,
        vec![DesiredEntry::create(
            "../destination/created",
            EntryKind::File,
        )],
    )
    .unwrap();

    plan.apply(DeletionMode::Permanent).unwrap();

    assert!(destination.join("created").is_file());
}

#[test]
fn an_existing_entry_can_move_outside_the_explorer_root() {
    let parent = TempDir::new("outside-move");
    let source = parent.path().join("source");
    let destination = parent.path().join("destination");
    fs::create_dir(&source).unwrap();
    fs::create_dir(&destination).unwrap();
    fs::write(source.join("original"), "content").unwrap();
    let snapshot = DirectorySnapshot::read(&source).unwrap();
    let original = snapshot.entries().first().unwrap();
    let plan = FsPlan::build(
        source.clone(),
        snapshot.clone(),
        vec![DesiredEntry::existing(original, "../destination/moved")],
    )
    .unwrap();

    plan.apply(DeletionMode::Permanent).unwrap();

    assert!(!source.join("original").exists());
    assert_eq!(
        fs::read_to_string(destination.join("moved")).unwrap(),
        "content"
    );
}

#[test]
fn a_repeated_identity_copies_a_file_outside_the_explorer_root() {
    let parent = TempDir::new("outside-copy");
    let source = parent.path().join("source");
    let destination = parent.path().join("destination");
    fs::create_dir(&source).unwrap();
    fs::create_dir(&destination).unwrap();
    fs::write(source.join("original"), "content").unwrap();
    let snapshot = DirectorySnapshot::read(&source).unwrap();
    let original = snapshot.entries().first().unwrap();
    let plan = FsPlan::build(
        source.clone(),
        snapshot.clone(),
        vec![
            DesiredEntry::existing(original, "original"),
            DesiredEntry::existing(original, "../destination/copied"),
        ],
    )
    .unwrap();
    assert!(matches!(
        plan.operations(),
        [FsOperation::Copy { from, to, .. }]
            if from == Path::new("original")
                && to == Path::new("../destination/copied")
    ));

    plan.apply(DeletionMode::Permanent).unwrap();

    assert_eq!(
        fs::read_to_string(source.join("original")).unwrap(),
        "content"
    );
    assert_eq!(
        fs::read_to_string(destination.join("copied")).unwrap(),
        "content"
    );
}

#[test]
fn copied_directories_include_nested_files() {
    let parent = TempDir::new("directory-copy");
    let source = parent.path().join("source");
    let destination = parent.path().join("destination");
    fs::create_dir(&source).unwrap();
    fs::create_dir(&destination).unwrap();
    fs::create_dir(source.join("tree")).unwrap();
    fs::create_dir(source.join("tree/nested")).unwrap();
    fs::write(source.join("tree/nested/file"), "nested content").unwrap();
    let snapshot = DirectorySnapshot::read(&source).unwrap();
    let tree = snapshot.entries().first().unwrap();
    let plan = FsPlan::build(
        source,
        snapshot.clone(),
        vec![
            DesiredEntry::existing(tree, "tree"),
            DesiredEntry::existing(tree, "../destination/tree-copy"),
        ],
    )
    .unwrap();

    plan.apply(DeletionMode::Permanent).unwrap();

    assert_eq!(
        fs::read_to_string(destination.join("tree-copy/nested/file")).unwrap(),
        "nested content"
    );
}

#[test]
fn a_moved_entry_can_be_copied_from_its_final_path() {
    let parent = TempDir::new("move-and-copy");
    let source = parent.path().join("source");
    let destination = parent.path().join("destination");
    fs::create_dir(&source).unwrap();
    fs::create_dir(&destination).unwrap();
    fs::write(source.join("original"), "content").unwrap();
    let snapshot = DirectorySnapshot::read(&source).unwrap();
    let original = snapshot.entries().first().unwrap();
    let plan = FsPlan::build(
        source.clone(),
        snapshot.clone(),
        vec![
            DesiredEntry::existing(original, "renamed"),
            DesiredEntry::existing(original, "../destination/copied"),
        ],
    )
    .unwrap();

    plan.apply(DeletionMode::Permanent).unwrap();

    assert!(!source.join("original").exists());
    assert_eq!(
        fs::read_to_string(source.join("renamed")).unwrap(),
        "content"
    );
    assert_eq!(
        fs::read_to_string(destination.join("copied")).unwrap(),
        "content"
    );
}

#[test]
fn a_directory_transfer_cannot_copy_a_parent_into_its_descendant() {
    let parent = TempDir::new("transfer-inside-itself");
    let source = parent.path().join("source");
    let destination = source.join("nested");
    fs::create_dir_all(&destination).unwrap();
    fs::write(source.join("content"), "content").unwrap();
    let snapshot = DirectorySnapshot::read(&destination).unwrap();

    let error = FsPlan::build(
        destination,
        snapshot,
        vec![DesiredEntry::transfer(
            source.clone(),
            "source-copy",
            EntryKind::Directory,
            TransferMode::Copy,
            SourceFingerprint::capture(&source).unwrap(),
        )],
    )
    .unwrap_err();

    assert!(error.to_string().contains("inside itself"));
}

#[test]
fn a_transfer_cannot_target_its_own_source_path() {
    let directory = TempDir::new("transfer-onto-itself");
    let source = directory.path().join("note");
    fs::write(&source, "content").unwrap();
    let snapshot = DirectorySnapshot::read(directory.path()).unwrap();
    let expected = SourceFingerprint::capture(&source).unwrap();

    for mode in [TransferMode::Copy, TransferMode::Move] {
        let error = FsPlan::build(
            directory.path().to_path_buf(),
            snapshot.clone(),
            vec![DesiredEntry::transfer(
                source.clone(),
                "note",
                EntryKind::File,
                mode,
                expected.clone(),
            )],
        )
        .unwrap_err();
        assert!(error.to_string().contains("onto itself"));
    }
    assert_eq!(fs::read_to_string(source).unwrap(), "content");
}

#[test]
fn a_transfer_copy_source_cannot_be_deleted_by_the_same_plan() {
    let directory = TempDir::new("copy-source-deleted");
    let source_directory = directory.path().join("source");
    let source = source_directory.join("note");
    fs::create_dir(&source_directory).unwrap();
    fs::write(&source, "content").unwrap();
    let snapshot = DirectorySnapshot::read(directory.path()).unwrap();

    let error = FsPlan::build(
        directory.path().to_path_buf(),
        snapshot,
        vec![DesiredEntry::transfer(
            source.clone(),
            "copied",
            EntryKind::File,
            TransferMode::Copy,
            SourceFingerprint::capture(&source).unwrap(),
        )],
    )
    .unwrap_err();

    assert!(error.to_string().contains("moved or deleted"));
    assert_eq!(fs::read_to_string(source).unwrap(), "content");
    assert!(!directory.path().join("copied").exists());
}

#[test]
fn a_changed_transfer_source_aborts_before_any_operation() {
    let parent = TempDir::new("changed-transfer-source");
    let source_directory = parent.path().join("source");
    let destination = parent.path().join("destination");
    let source = source_directory.join("note");
    fs::create_dir(&source_directory).unwrap();
    fs::create_dir(&destination).unwrap();
    fs::write(&source, "content").unwrap();
    let expected = SourceFingerprint::capture(&source).unwrap();
    let snapshot = DirectorySnapshot::read(&destination).unwrap();
    let plan = FsPlan::build(
        destination.clone(),
        snapshot,
        vec![DesiredEntry::transfer(
            source.clone(),
            "note",
            EntryKind::File,
            TransferMode::Copy,
            expected,
        )],
    )
    .unwrap();
    fs::remove_file(&source).unwrap();
    fs::create_dir(&source).unwrap();

    let error = plan.apply(DeletionMode::Permanent).unwrap_err();

    assert!(error.report.applied.is_empty());
    assert!(error.to_string().contains("source changed on disk"));
    assert!(!destination.join("note").exists());
    assert!(source.is_dir());
}

#[cfg(unix)]
#[test]
fn a_symlink_cannot_be_used_as_a_target_parent() {
    use std::os::unix::fs::symlink;

    let directory = TempDir::new("symlink-parent");
    let outside = TempDir::new("outside");
    symlink(outside.path(), directory.path().join("link")).unwrap();
    let snapshot = DirectorySnapshot::read(directory.path()).unwrap();
    let link = snapshot.entries().first().unwrap();
    let plan = FsPlan::build(
        directory.path().to_path_buf(),
        snapshot.clone(),
        vec![
            DesiredEntry::existing(link, "link"),
            DesiredEntry::create("link/outside", EntryKind::File),
        ],
    )
    .unwrap();

    let error = plan.apply(DeletionMode::Permanent).unwrap_err();

    assert!(error.to_string().contains("not a real directory"));
    assert!(!outside.path().join("outside").exists());
}

#[cfg(unix)]
#[test]
fn escaping_the_root_does_not_allow_a_symlink_target_parent() {
    use std::os::unix::fs::symlink;

    let parent = TempDir::new("outside-symlink-parent");
    let source = parent.path().join("source");
    let outside = parent.path().join("outside");
    fs::create_dir(&source).unwrap();
    fs::create_dir(&outside).unwrap();
    symlink(&outside, parent.path().join("link")).unwrap();
    let snapshot = DirectorySnapshot::read(&source).unwrap();
    let plan = FsPlan::build(
        source,
        snapshot,
        vec![DesiredEntry::create("../link/created", EntryKind::File)],
    )
    .unwrap();

    let error = plan.apply(DeletionMode::Permanent).unwrap_err();

    assert!(error.to_string().contains("not a real directory"));
    assert!(!outside.join("created").exists());
}

#[cfg(unix)]
#[test]
fn copying_a_symlink_preserves_the_link_instead_of_its_target() {
    use std::os::unix::fs::symlink;

    let parent = TempDir::new("copy-symlink");
    let source = parent.path().join("source");
    let destination = parent.path().join("destination");
    fs::create_dir(&source).unwrap();
    fs::create_dir(&destination).unwrap();
    fs::write(source.join("target"), "content").unwrap();
    symlink("target", source.join("link")).unwrap();
    let snapshot = DirectorySnapshot::read(&source).unwrap();
    let link = snapshot
        .entries()
        .iter()
        .find(|entry| entry.path == Path::new("link"))
        .unwrap();
    let desired = snapshot
        .entries()
        .iter()
        .map(|entry| DesiredEntry::existing(entry, entry.path.clone()))
        .chain(std::iter::once(DesiredEntry::existing(
            link,
            "../destination/link-copy",
        )))
        .collect();
    let plan = FsPlan::build(source, snapshot.clone(), desired).unwrap();

    plan.apply(DeletionMode::Permanent).unwrap();

    let copied = destination.join("link-copy");
    assert!(
        fs::symlink_metadata(&copied)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(fs::read_link(copied).unwrap(), Path::new("target"));
}

#[test]
fn permanent_deletion_is_an_explicit_apply_mode() {
    let directory = TempDir::new("delete");
    fs::write(directory.path().join("gone"), "content").unwrap();
    let snapshot = DirectorySnapshot::read(directory.path()).unwrap();
    let plan = FsPlan::build(directory.path().to_path_buf(), snapshot, Vec::new()).unwrap();
    assert!(matches!(
        plan.operations(),
        [FsOperation::Delete { path, .. }] if path == Path::new("gone")
    ));

    plan.apply(DeletionMode::Permanent).unwrap();

    assert!(!directory.path().join("gone").exists());
}
