// SPDX-License-Identifier: MPL-2.0

use super::*;

#[test]
fn short_name_alias_cannot_copy_a_directory_inside_itself() {
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    use windows_sys::Win32::Storage::FileSystem::GetShortPathNameW;
    let root = TempDir::new();
    let source = root.join("long directory name");
    fs::create_dir(&source).unwrap();
    let wide: Vec<_> = source.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut short = vec![0; 32768];
    let count = unsafe { GetShortPathNameW(wide.as_ptr(), short.as_mut_ptr(), short.len() as u32) };
    assert!(count > 0 && count < short.len() as u32);
    let alias = PathBuf::from(std::ffi::OsString::from_wide(&short[..count as usize]));
    // Filesystems with 8.3 names disabled return the original spelling; the
    // same containment invariant still applies without changing volume policy.
    let target = PathBuf::from(alias.file_name().unwrap()).join("child");
    let snapshot = DirectorySnapshot::read(&root.0).unwrap();
    let entry = &snapshot.entries()[0];
    let desired = vec![
        DesiredEntry::existing(entry, "long directory name"),
        DesiredEntry::existing(entry, target),
    ];
    assert!(FsPlan::build(root.0.clone(), snapshot, desired).is_err());
    assert_eq!(fs::read_dir(source).unwrap().count(), 0);
}

#[test]
fn directory_case_alias_cannot_be_copied_or_moved_inside_itself() {
    let root = TempDir::new();
    fs::create_dir(root.join("source")).unwrap();
    for copy in [false, true] {
        let snapshot = DirectorySnapshot::read(&root.0).unwrap();
        let entry = &snapshot.entries()[0];
        let mut desired = vec![DesiredEntry::existing(entry, "SOURCE/child")];
        if copy {
            desired.push(DesiredEntry::existing(entry, "source"));
        }
        assert!(FsPlan::build(root.0.clone(), snapshot, desired).is_err());
        assert_eq!(fs::read_dir(root.join("source")).unwrap().count(), 0);
    }
}

#[test]
fn occupied_hardlink_refuses_whole_plan_before_unrelated_delete() {
    let root = TempDir::new();
    fs::write(root.join("source"), "keep").unwrap();
    fs::write(root.join("delete-me"), "also keep").unwrap();
    fs::create_dir(root.join("occupied")).unwrap();
    fs::hard_link(root.join("source"), root.join("occupied/alias")).unwrap();
    let snapshot = DirectorySnapshot::read(&root.0).unwrap();
    let desired = snapshot
        .entries()
        .iter()
        .filter(|entry| entry.path != Path::new("delete-me"))
        .map(|entry| {
            DesiredEntry::existing(
                entry,
                if entry.path == Path::new("source") {
                    PathBuf::from("occupied/alias")
                } else {
                    entry.path.clone()
                },
            )
        })
        .collect();
    let plan = FsPlan::build(root.0.clone(), snapshot, desired).unwrap();
    let error = apply(&plan, |_, _, _| Ok(())).unwrap_err();
    assert!(
        error.to_string().contains("target already exists"),
        "{error}"
    );
    assert_eq!(
        fs::read_to_string(root.join("delete-me")).unwrap(),
        "also keep"
    );
    assert_eq!(fs::read_to_string(root.join("source")).unwrap(), "keep");
    assert_eq!(
        fs::read_to_string(root.join("occupied/alias")).unwrap(),
        "keep"
    );
    assert_eq!(fs::read_dir(&root.0).unwrap().count(), 3);
}

#[test]
fn native_exclusive_rename_preserves_files_and_directories() {
    for directory in [false, true] {
        let root = TempDir::new();
        fs::write(root.join("source"), "source bytes").unwrap();
        if directory {
            fs::create_dir(root.join("target")).unwrap();
        } else {
            fs::write(root.join("target"), "target bytes").unwrap();
        }
        assert!(platform::rename_noreplace(&root.join("source"), &root.join("target")).is_err());
        assert_eq!(
            fs::read_to_string(root.join("source")).unwrap(),
            "source bytes"
        );
        if directory {
            assert!(root.join("target").is_dir());
        } else {
            assert_eq!(
                fs::read_to_string(root.join("target")).unwrap(),
                "target bytes"
            );
        }
    }
}

#[test]
fn case_only_rename_keeps_entry_and_contents() {
    let root = TempDir::new();
    fs::write(root.join("notes.txt"), "café\r\n").unwrap();
    let identity = crate::windows_fs::Identity::read(&root.join("notes.txt")).unwrap();
    let snapshot = DirectorySnapshot::read(&root.0).unwrap();
    let desired = vec![DesiredEntry::existing(&snapshot.entries()[0], "NOTES.txt")];
    let plan = FsPlan::build(root.0.clone(), snapshot, desired).unwrap();
    apply(&plan, |_, _, _| Ok(())).unwrap();
    assert_eq!(
        fs::read_to_string(root.join("NOTES.txt")).unwrap(),
        "café\r\n"
    );
    assert_eq!(
        crate::windows_fs::Identity::read(&root.join("NOTES.txt")).unwrap(),
        identity
    );
    assert_eq!(
        fs::read_dir(&root.0)
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .file_name(),
        "NOTES.txt"
    );
}

#[test]
fn aliases_and_invalid_names_are_refused_before_mutation() {
    let root = TempDir::new();
    fs::write(root.join("source"), "keep").unwrap();
    for name in [
        "NUL",
        "con.txt",
        "COM¹.txt",
        "folder/LPT9",
        "trailing.",
        "trailing ",
        "ads:stream",
        "C:relative",
        "\\rooted",
        "bad?name",
    ] {
        let snapshot = DirectorySnapshot::read(&root.0).unwrap();
        let desired = vec![DesiredEntry::existing(&snapshot.entries()[0], name)];
        assert!(
            FsPlan::build(root.0.clone(), snapshot, desired).is_err(),
            "{name}"
        );
        assert_eq!(fs::read_to_string(root.join("source")).unwrap(), "keep");
    }
    let snapshot = DirectorySnapshot::read(&root.0).unwrap();
    let desired = vec![
        DesiredEntry::existing(&snapshot.entries()[0], "Name"),
        DesiredEntry::existing(&snapshot.entries()[0], "name"),
    ];
    assert!(FsPlan::build(root.0.clone(), snapshot, desired).is_err());
    assert_eq!(fs::read_dir(&root.0).unwrap().count(), 1);
}

#[test]
fn replaced_staging_directory_is_never_cleaned_as_owned() {
    let root = TempDir::new();
    let mut io = ApplyIo::default();
    let tree = OwnedTree::allocate(&root.0, "identity-test", &mut io).unwrap();
    let retained = root.join("retained");
    fs::rename(&tree.root, &retained).unwrap();
    fs::create_dir(&tree.root).unwrap();
    fs::write(tree.root.join("foreign"), "keep").unwrap();
    assert!(tree.cleanup(&mut io).is_err());
    assert_eq!(
        fs::read_to_string(tree.root.join("foreign")).unwrap(),
        "keep"
    );
    assert!(retained.is_dir());
}

#[test]
fn same_size_and_timestamp_replacement_changes_source_fingerprint() {
    let root = TempDir::new();
    let path = root.join("source");
    fs::write(&path, "first").unwrap();
    let modified = fs::metadata(&path).unwrap().modified().unwrap();
    let first = SourceFingerprint::capture(&path).unwrap();
    fs::rename(&path, root.join("retained")).unwrap();
    fs::write(&path, "other").unwrap();
    let file = fs::OpenOptions::new().write(true).open(&path).unwrap();
    file.set_times(fs::FileTimes::new().set_modified(modified))
        .unwrap();
    assert_ne!(SourceFingerprint::capture(&path).unwrap(), first);
}
