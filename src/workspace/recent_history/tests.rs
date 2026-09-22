// SPDX-License-Identifier: MPL-2.0

use super::*;
use crate::test_support::TestRuntimeRoot;
use std::fs;

fn fixture() -> (TestRuntimeRoot, PathBuf) {
    let root = TestRuntimeRoot::new("history-semantics").unwrap();
    let path = root.join("cache/workspaces.json");
    (root, path)
}

fn project(root: &TestRuntimeRoot, name: &str) -> PathBuf {
    let path = root.join(name);
    fs::create_dir_all(&path).unwrap();
    path.canonicalize().unwrap()
}

fn entry(path: PathBuf, number: Option<u8>) -> RecentEntry {
    RecentEntry::new(path, Some("workspace".into()), number, None)
}

#[test]
fn visits_keep_names_and_digits_but_lifecycle_ensure_does_not_reorder_or_activate() {
    let (root, path) = fixture();
    let first = project(&root, "one/same");
    let second = project(&root, "two/same");
    let original = record_recent_workspace_name_in(&path, &first)
        .unwrap()
        .unwrap();
    let other = record_recent_workspace_name_in(&path, &second)
        .unwrap()
        .unwrap();
    assert_eq!((original.name.as_str(), original.number), ("same", Some(1)));
    assert_eq!((other.name.as_str(), other.number), ("same-2", Some(2)));
    let before = read_recents(Some(&path)).unwrap();
    ensure_recent_workspace_in(&path, &first).unwrap();
    assert_eq!(read_recents(Some(&path)).unwrap(), before);
    record_recent_workspace_name_in(&path, &first).unwrap();
    let visited = read_recents(Some(&path)).unwrap();
    assert_eq!(visited[0].project_root, first);
    assert_eq!(visited[0].number, Some(1));
    assert_eq!(visited[0].name.as_deref(), Some("same"));
    assert!(
        visited
            .iter()
            .all(|row| row.last_active_unix_seconds.is_none())
    );
    record_workspace_activity_in(&path, &second).unwrap();
    let activated = read_recents(Some(&path)).unwrap();
    assert_eq!(activated[0].project_root, second);
    assert!(activated[0].last_active_unix_seconds.is_some());
    assert!(activated[1].last_active_unix_seconds.is_none());
}

#[test]
fn renumber_swaps_pins_and_explicitly_cleared_digits_survive_a_visit() {
    let (root, path) = fixture();
    let first = project(&root, "first");
    let second = project(&root, "second");
    for project in [&first, &second] {
        record_recent_workspace_name_in(&path, project).unwrap();
    }
    assert_eq!(
        set_recent_workspace_number_in(Some(&path), &first, Some(2)).unwrap(),
        Some(second.clone())
    );
    let rows = read_recents(Some(&path)).unwrap();
    assert_eq!(
        rows.iter()
            .find(|row| row.project_root == first)
            .unwrap()
            .number,
        Some(2)
    );
    assert_eq!(
        rows.iter()
            .find(|row| row.project_root == second)
            .unwrap()
            .number,
        Some(1)
    );
    assert!(
        rows.iter()
            .find(|row| row.project_root == first)
            .unwrap()
            .number_pinned
    );
    set_recent_workspace_number_in(Some(&path), &first, None).unwrap();
    record_recent_workspace_name_in(&path, &first).unwrap();
    let rows = read_recents(Some(&path)).unwrap();
    assert!(rows[0].number_declined);
    assert!(!rows[0].number_pinned);
    assert_eq!(rows[0].number, None);
    let before = fs::read(&path).unwrap();
    assert!(set_recent_workspace_number_in(Some(&path), &first, Some(10)).is_err());
    assert_eq!(fs::read(&path).unwrap(), before);
}

#[test]
fn clearing_rows_preserves_later_records_and_vanished_project_history() {
    let (root, path) = fixture();
    let first = project(&root, "first");
    let vanished = project(&root, "vanished");
    for project in [&first, &vanished] {
        record_recent_workspace_name_in(&path, project).unwrap();
    }
    let snapshot = read_recents(Some(&path)).unwrap();
    fs::remove_dir(&vanished).unwrap();
    let later = project(&root, "later");
    record_recent_workspace_name_in(&path, &later).unwrap();
    assert_eq!(
        clear_recent_workspaces_in(Some(&path), &[snapshot[1].project_root.clone()]).unwrap(),
        1
    );
    let rows = read_recents(Some(&path)).unwrap();
    assert_eq!(
        rows.iter().map(|row| &row.project_root).collect::<Vec<_>>(),
        vec![&later, &vanished]
    );
    assert_eq!(listable_recents(rows).len(), 1);
    assert!(forget_recent_workspace_in(Some(&path), &vanished).unwrap());
    assert!(!forget_recent_workspace_in(Some(&path), &vanished).unwrap());
}

#[test]
fn historical_fields_default_and_duplicate_digits_are_repaired_without_loss() {
    let (root, _) = fixture();
    let first = project(&root, "first");
    let second = project(&root, "second");
    let bytes = serde_json::to_vec(&serde_json::json!([
        { "project_root_bytes": encode_path(&first) },
        { "project_root_bytes": encode_path(&second), "number": 1, "number_pinned": true },
        { "project_root_bytes": encode_path(&first.join("missing")), "number": 1, "number_pinned": true }
    ])).unwrap();
    let mut entries = decode_recents(&bytes).unwrap();
    assert_eq!(entries[0].name, None);
    assert_eq!(entries[0].last_active_unix_seconds, None);
    assert!(!entries[0].number_declined);
    assert_eq!(entries[1].number, Some(1));
    assert_eq!(entries[2].number, None);
    assert!(!entries[2].number_pinned);
    assign_missing_default_workspace_names(&mut entries);
    assign_missing_default_workspace_numbers(&mut entries);
    assert_eq!(
        entries.iter().map(|row| row.number).collect::<Vec<_>>(),
        vec![Some(2), Some(1), Some(3)]
    );
    assert_eq!(
        decode_recents(&encode_recents(&entries).unwrap()).unwrap(),
        entries
    );
}

#[test]
fn names_are_logical_utf8_values_and_only_nine_default_digits_are_allocated() {
    let (root, path) = fixture();
    for index in 0..10 {
        record_recent_workspace_name_in(&path, &project(&root, &format!("p-{index}"))).unwrap();
    }
    let rows = read_recents(Some(&path)).unwrap();
    assert_eq!(rows.iter().filter(|row| row.number.is_some()).count(), 9);
    assert_eq!(rows[0].number, None);
    assert_eq!(
        normalize_session_name("  release candidate  "),
        "release-candidate"
    );
    assert!(validate_host_name("release/candidate:α").is_ok());
    for name in ["", " padded", "padded ", "line\nbreak"] {
        assert!(validate_host_name(name).is_err());
    }
    assert!(validate_host_name(&"é".repeat(33)).is_err());
    let first = root.join("é".repeat(40));
    let name = unique_default_workspace_name(&first, &[]);
    assert!(name.len() <= MAX_HOST_NAME_BYTES);
    let previous = RecentEntry::new(first.clone(), Some(name.clone()), None, None);
    let second = unique_default_workspace_name(&first, &[previous]);
    assert!(second.ends_with("-2"));
    assert!(second.len() <= MAX_HOST_NAME_BYTES);
    assert!(validate_host_name(&second).is_ok());
    rename_recent_workspace_in(&path, &rows[0].project_root, "Explicit Name").unwrap();
    record_recent_workspace_name_in(&path, &rows[0].project_root).unwrap();
    assert_eq!(
        read_recents(Some(&path)).unwrap()[0].name.as_deref(),
        Some("Explicit Name")
    );
}

#[test]
fn malformed_and_bounded_history_is_refused_without_mutating_existing_content() {
    let (root, path) = fixture();
    let first = project(&root, "first");
    record_recent_workspace_name_in(&path, &first).unwrap();
    let mut storage = storage::LockedHistory::acquire(&path).unwrap();
    storage.write(b"not json").unwrap();
    drop(storage);
    assert!(read_recents(Some(&path)).is_err());
    assert!(record_recent_workspace_name_in(&path, &first).is_err());
    assert_eq!(fs::read(&path).unwrap(), b"not json");
    let original = entry(first, None);
    assert!(encode_recents(&vec![original.clone(); RECENT_LIMIT + 1]).is_err());
    assert!(decode_recents(&vec![b' '; MAX_RECENTS_BYTES + 1]).is_err());
    let invalid =
        serde_json::json!([{ "project_root_bytes": vec![1; MAX_PERSISTED_PATH_BYTES + 1] }]);
    assert!(decode_recents(&serde_json::to_vec(&invalid).unwrap()).is_err());
    assert!(encode_recents(&[entry(PathBuf::from("relative"), None)]).is_err());
}

#[test]
fn unusable_optional_cache_disables_history_without_touching_existing_file() {
    let (root, _) = fixture();
    let blocked = root.join("file");
    fs::write(&blocked, b"preserve").unwrap();
    assert_eq!(recent_file_in(Some(blocked.join("cache"))), None);
    assert_eq!(fs::read(blocked).unwrap(), b"preserve");
    assert!(read_recents(None).unwrap().is_empty());
    assert!(!forget_recent_workspace_in(None, root.path()).unwrap());
}

#[cfg(windows)]
#[test]
fn native_paths_preserve_unpaired_utf16_and_reject_odd_or_nul_encodings() {
    use std::{ffi::OsString, os::windows::ffi::OsStringExt};
    let units = [b'C' as u16, b':' as u16, b'\\' as u16, 0xd800, b'x' as u16];
    let path = PathBuf::from(OsString::from_wide(&units));
    let bytes = encode_recents(&[entry(path.clone(), Some(1))]).unwrap();
    assert_eq!(decode_recents(&bytes).unwrap()[0].project_root, path);
    let encoded = encode_path(Path::new("C:\\ordinary"));
    assert!(encoded.contains(&0));
    assert!(validate_persisted_path(&encoded, "path").is_ok());
    let mut odd = encoded.clone();
    odd.pop();
    assert!(validate_persisted_path(&odd, "path").is_err());
    let mut nul = encoded;
    nul.extend_from_slice(&[0, 0]);
    assert!(validate_persisted_path(&nul, "path").is_err());
}

#[cfg(unix)]
#[test]
fn unix_paths_preserve_raw_bytes_and_reject_nul() {
    use std::{ffi::OsString, os::unix::ffi::OsStringExt};
    let path = PathBuf::from(OsString::from_vec(b"/raw-\xff".to_vec()));
    let value = entry(path.clone(), Some(1));
    assert_eq!(
        decode_recents(&encode_recents(&[value]).unwrap()).unwrap()[0].project_root,
        path
    );
    assert!(validate_persisted_path(b"/bad\0path", "path").is_err());
    assert_eq!(encode_path(Path::new("/fixed")), b"/fixed");
}
