// SPDX-License-Identifier: MPL-2.0

use super::*;

#[test]
fn indexed_planning_preserves_original_and_first_duplicate_primary_rows() {
    let root = TempDir::new();
    for name in ["a", "b", "c", "d"] {
        fs::write(root.join(name), name).unwrap();
    }
    let snapshot = DirectorySnapshot::read(&root.0).unwrap();
    let [a, b, c, _] = snapshot.entries() else {
        panic!("four entries")
    };
    let desired = vec![
        DesiredEntry::existing(c, "c"),
        DesiredEntry::existing(b, "z"),
        DesiredEntry::existing(a, "a-copy"),
        DesiredEntry::existing(b, "b-copy"),
        DesiredEntry::existing(a, "a"),
    ];
    let plan = FsPlan::build(root.0.clone(), snapshot.clone(), desired).unwrap();
    assert_eq!(
        plan.operations(),
        &[
            FsOperation::Rename {
                from: "b".into(),
                to: "z".into(),
                kind: EntryKind::File
            },
            FsOperation::Copy {
                from: "a".into(),
                to: "a-copy".into(),
                kind: EntryKind::File
            },
            FsOperation::Copy {
                from: "z".into(),
                to: "b-copy".into(),
                kind: EntryKind::File
            },
            FsOperation::Delete {
                path: "d".into(),
                kind: EntryKind::File
            },
        ]
    );
    plan.apply_with_trash(DeletionMode::Permanent, &NoTrash)
        .unwrap();
    assert_eq!(fs::read_to_string(root.join("a-copy")).unwrap(), "a");
    assert_eq!(fs::read_to_string(root.join("b-copy")).unwrap(), "b");
    assert!(!root.join("b").exists());
    assert!(!root.join("d").exists());

    for id in [0, 5, u64::MAX] {
        assert!(snapshot.entry(EntryId::new(id)).is_none());
        let stale = DesiredEntry::identified(EntryId::new(id), "unknown", EntryKind::File);
        assert!(
            FsPlan::build(root.0.clone(), snapshot.clone(), vec![stale])
                .unwrap_err()
                .to_string()
                .contains("identity is stale")
        );
    }
}

#[test]
fn unchanged_reordered_large_directory_plans_no_operations() {
    let root = TempDir::new();
    for index in 0..2048 {
        fs::write(root.join(format!("file-{index:04}")), "").unwrap();
    }
    let snapshot = DirectorySnapshot::read(&root.0).unwrap();
    let desired = snapshot
        .entries()
        .iter()
        .rev()
        .map(|entry| DesiredEntry::existing(entry, entry.path.clone()))
        .collect();
    assert!(
        FsPlan::build(root.0.clone(), snapshot, desired)
            .unwrap()
            .is_empty()
    );
}
