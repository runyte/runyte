// SPDX-License-Identifier: MPL-2.0

use super::*;
use crate::{
    test_support::TestRuntimeRoot,
    workspace::recent_history::{RecentEntry, decode_recents, encode_recents},
};
use std::{
    fs,
    os::windows::process::CommandExt,
    process::{Child, Command, Stdio},
};
use windows_sys::Win32::System::Threading::{CREATE_NO_WINDOW, WaitForSingleObject};

fn fixture() -> (TestRuntimeRoot, std::path::PathBuf) {
    let root = TestRuntimeRoot::new("native-history-storage").unwrap();
    let path = root.join("cache/workspaces.json");
    (root, path)
}

#[test]
fn transaction_uses_directory_pinned_before_lock_acquisition_after_path_replacement() {
    let (root, path) = fixture();
    let directory = Directory::open(path.parent().unwrap(), true).unwrap();
    directory
        .atomic_write(OsStr::new("workspaces.json"), b"[1]")
        .unwrap();
    let moved = root.join("old-cache");
    // Windows can refuse a directory rename while a child file is open.
    // Replace the path in the admission gap before opening the stable lock.
    fs::rename(path.parent().unwrap(), &moved).unwrap();
    let replacement = Directory::open(path.parent().unwrap(), true).unwrap();
    replacement
        .atomic_write(OsStr::new("workspaces.json"), b"replacement")
        .unwrap();
    let lock_name = OsString::from("workspaces.lock");
    let file = directory.append(&lock_name).unwrap();
    assert!(FileLock::try_lock(&file).unwrap());
    let mut transaction = LockedHistory {
        directory,
        name: OsString::from("workspaces.json"),
        lock_name,
        lock: FileLock(file),
    };
    assert_eq!(transaction.read().unwrap(), b"[1]");
    transaction.write(b"[2]").unwrap();
    assert_eq!(fs::read(moved.join("workspaces.json")).unwrap(), b"[2]");
    assert_eq!(fs::read(&path).unwrap(), b"replacement");
}

#[test]
fn replaced_lock_is_refused_before_transaction_mutation() {
    let (_root, path) = fixture();
    let mut transaction = LockedHistory::acquire(&path).unwrap();
    transaction.write(b"[]").unwrap();
    transaction
        .directory
        .atomic_write(&transaction.lock_name, b"replacement lock")
        .unwrap();
    assert!(transaction.read().is_err());
    assert!(transaction.write(b"changed").is_err());
    assert_eq!(fs::read(&path).unwrap(), b"[]");
}

#[test]
fn bounded_contention_preserves_history_and_release_allows_the_next_writer() {
    let (_root, path) = fixture();
    let mut first = LockedHistory::acquire(&path).unwrap();
    first.write(b"[]").unwrap();
    let error = LockedHistory::acquire_with_budget(&path, Duration::ZERO)
        .err()
        .unwrap();
    assert_eq!(
        error.downcast_ref::<io::Error>().unwrap().kind(),
        io::ErrorKind::WouldBlock
    );
    assert_eq!(read(&path).unwrap(), b"[]");
    drop(first);
    let mut second = LockedHistory::acquire(&path).unwrap();
    second.write(b"[2]").unwrap();
    assert_eq!(read(&path).unwrap(), b"[2]");
}

#[test]
fn atomic_replacement_preserves_an_open_reader_and_refuses_hardlinked_content() {
    let (root, path) = fixture();
    let mut transaction = LockedHistory::acquire(&path).unwrap();
    transaction.write(b"old").unwrap();
    let mut previous = transaction.directory.open_read(&transaction.name).unwrap();
    transaction.write(b"new").unwrap();
    let mut bytes = Vec::new();
    previous.read_to_end(&mut bytes).unwrap();
    assert_eq!(bytes, b"old");
    assert_eq!(read(&path).unwrap(), b"new");
    drop((previous, transaction));
    let alias = root.join("hardlink");
    fs::hard_link(&path, &alias).unwrap();
    assert!(read(&path).is_err());
    assert!(LockedHistory::acquire(&path).unwrap().read().is_err());
    assert_eq!(fs::read(alias).unwrap(), b"new");
}

#[test]
fn native_storage_refuses_case_insensitive_lock_alias() {
    let (root, _) = fixture();
    assert!(LockedHistory::acquire(&root.join("WORKSPACES.LoCk")).is_err());
    assert!(!root.join("WORKSPACES.lock").exists());
}

const HELPER_ROOT: &str = "RUNYTE_HISTORY_STORAGE_FIXTURE";
const HELPER_NAME: &str =
    "workspace::recent_history::storage::tests::native_history_writer_fixture";

#[test]
#[ignore = "compiled helper for the native cross-process history lock test"]
fn native_history_writer_fixture() {
    let Some(root) = std::env::var_os(HELPER_ROOT).map(std::path::PathBuf::from) else {
        return;
    };
    let directory = Directory::open_existing(&root, true).unwrap();
    let cache = Directory::open_existing(&root.join("cache"), true).unwrap();
    let contender = cache.append(OsStr::new("workspaces.lock")).unwrap();
    assert!(!FileLock::try_lock(&contender).unwrap());
    drop(contender);
    directory
        .atomic_write(OsStr::new("ready"), b"observed locked")
        .unwrap();
    // The extended fixture deadline is not the production storage budget.
    let mut transaction = LockedHistory::acquire_with_budget(
        &root.join("cache/workspaces.json"),
        Duration::from_secs(15),
    )
    .unwrap();
    let mut rows = decode_recents(&transaction.read().unwrap()).unwrap();
    rows.push(RecentEntry::new(
        root.join("second"),
        Some("second".into()),
        Some(2),
        None,
    ));
    transaction.write(&encode_recents(&rows).unwrap()).unwrap();
}

struct FixtureChild(Child);
impl Drop for FixtureChild {
    fn drop(&mut self) {
        if self.0.try_wait().ok().flatten().is_none() {
            let _ = self.0.kill();
            // Even a failed termination request must not turn a fixture
            // assertion into an unbounded wait during unwinding.
            unsafe {
                WaitForSingleObject(self.0.as_raw_handle(), 5000);
            }
        }
    }
}

#[test]
fn independent_process_reads_the_latest_history_only_after_lock_release() {
    let (root, path) = fixture();
    let directory = Directory::open(root.path(), true).unwrap();
    let mut transaction = LockedHistory::acquire(&path).unwrap();
    let initial = RecentEntry::new(root.join("first"), Some("first".into()), Some(1), None);
    transaction
        .write(&encode_recents(std::slice::from_ref(&initial)).unwrap())
        .unwrap();
    let mut child = FixtureChild(
        Command::new(std::env::current_exe().unwrap())
            .args(["--exact", HELPER_NAME, "--ignored", "--nocapture"])
            .env(HELPER_ROOT, root.path())
            .env("XDG_CONFIG_HOME", root.join("config"))
            .creation_flags(CREATE_NO_WINDOW)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        match directory.open_read(OsStr::new("ready")) {
            Ok(_) => break,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => panic!("cannot read helper readiness: {error}"),
        }
        assert!(
            child.0.try_wait().unwrap().is_none(),
            "helper exited before observing the lock"
        );
        assert!(Instant::now() < deadline, "helper did not observe the lock");
        std::thread::sleep(Duration::from_millis(5));
    }
    // Make a newer update while the child is blocked. Its transaction must
    // read this version after release, not an earlier unlocked snapshot.
    let mut updated = initial;
    updated.name = Some("renamed by parent".into());
    transaction
        .write(&encode_recents(std::slice::from_ref(&updated)).unwrap())
        .unwrap();
    drop(transaction);
    let status = loop {
        if let Some(status) = child.0.try_wait().unwrap() {
            break status;
        }
        assert!(
            Instant::now() < deadline,
            "helper did not finish after lock release"
        );
        std::thread::sleep(Duration::from_millis(5));
    };
    assert!(status.success(), "history helper failed with {status}");
    let rows = decode_recents(&read(&path).unwrap()).unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0], updated);
    assert_eq!(rows[1].name.as_deref(), Some("second"));
}
