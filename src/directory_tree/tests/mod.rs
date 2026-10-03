// SPDX-License-Identifier: MPL-2.0

use super::*;

#[test]
fn applied_operations_keep_cached_tree_rows_in_navigation_order() {
    let root = crate::test_support::TestRuntimeRoot::new("tree").unwrap();
    let project = root.path().join("project");
    fs::create_dir(&project).unwrap();
    fs::create_dir(project.join("z-dir")).unwrap();
    fs::write(project.join("z-file"), "").unwrap();
    let mut tree = DirectoryTree::new(project.clone());
    tree.expanded.insert(project.clone());
    tree.listings
        .insert(project.clone(), read_directory(&project).unwrap());

    tree.note_applied(
        &project,
        &crate::fs_plan::ApplyReport {
            applied: vec![
                FsOperation::Create {
                    path: "b-file".into(),
                    kind: EntryKind::File,
                },
                FsOperation::Create {
                    path: "a-dir".into(),
                    kind: EntryKind::Directory,
                },
                FsOperation::Rename {
                    from: "z-file".into(),
                    to: "a-file".into(),
                    kind: EntryKind::File,
                },
            ],
            ..Default::default()
        },
    );

    let expected = ["a-dir", "z-dir", "a-file", "b-file"];
    assert_eq!(
        tree.listings[&project]
            .iter()
            .map(|entry| entry.path.file_name().unwrap().to_str().unwrap())
            .collect::<Vec<_>>(),
        expected
    );
    assert_eq!(
        tree.rows()
            .iter()
            .skip(1)
            .map(|row| row.path.file_name().unwrap().to_str().unwrap().to_owned())
            .collect::<Vec<_>>(),
        expected
    );
    tree.select_relative(1);
    assert_eq!(tree.selected, project.join("a-dir"));
    tree.select_last();
    assert_eq!(tree.selected, project.join("b-file"));
}
