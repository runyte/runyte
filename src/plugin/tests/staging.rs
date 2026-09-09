// SPDX-License-Identifier: MPL-2.0

use super::*;
use crate::fs_plan::{DeletionMode, DirectorySnapshot, FsOperation};
use std::{
    fs,
    path::PathBuf,
    sync::atomic::AtomicU64,
    time::{Duration, Instant},
};

struct Root(PathBuf);
impl Root {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "runyte-download-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path.canonicalize().unwrap())
    }
    fn directory(&self) -> filesystem::Directory {
        filesystem::Directory {
            path: self.0.clone(),
            snapshot: DirectorySnapshot::read_bounded(&self.0, true, 1024).unwrap(),
            revision: "test".into(),
        }
    }
}
impl Drop for Root {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn absent(path: &Path) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while path.exists() {
        assert!(
            Instant::now() < deadline,
            "private file cleanup did not finish"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn binary_download_plan_seals_bytes_against_original_open_writer_and_retains_clones() {
    let root = Root::new();
    let cancel = AtomicBool::new(false);
    let bytes = [0, 255, 12, 0, 128];
    let download = Download::create(&root.0.join(".runyte"), bytes.len(), &cancel).unwrap();
    assert_eq!(fs::metadata(download.path()).unwrap().len(), 0);
    let original = download.path().to_owned();
    fs::write(&original, bytes).unwrap();
    let mut held_writer = fs::OpenOptions::new().write(true).open(&original).unwrap();
    let plan = download
        .prepare(
            &root.0,
            &root.directory(),
            "saved.bin",
            &crate::hash::sha256_hex(&bytes),
            &cancel,
        )
        .unwrap();
    assert_eq!(plan.lines(), ["copy downloaded file → saved.bin"]);
    let FsOperation::Copy { from, .. } = &plan.operations()[0] else {
        panic!("expected copy")
    };
    let sealed = plan.root().join(from);
    assert_ne!(sealed, original);
    held_writer.write_all(&[7; 5]).unwrap();
    drop(download);
    absent(&original);
    let worker_plan = plan.clone();
    drop(plan);
    assert_eq!(fs::read(&sealed).unwrap(), bytes);
    worker_plan.apply(DeletionMode::Permanent).unwrap();
    assert_eq!(fs::read(root.0.join("saved.bin")).unwrap(), bytes);
    drop(worker_plan);
    absent(&sealed);
}

#[test]
fn empty_download_creates_empty_destination_and_existing_or_late_destinations_refuse() {
    let root = Root::new();
    let cancel = AtomicBool::new(false);
    let download = Download::create(&root.0.join(".runyte"), 0, &cancel).unwrap();
    let digest = crate::hash::sha256_hex(&[]);
    let plan = download
        .prepare(&root.0, &root.directory(), "empty.bin", &digest, &cancel)
        .unwrap();
    plan.apply(DeletionMode::Permanent).unwrap();
    assert!(fs::read(root.0.join("empty.bin")).unwrap().is_empty());
    assert_eq!(
        download
            .prepare(&root.0, &root.directory(), "empty.bin", &digest, &cancel)
            .unwrap_err()
            .code,
        ErrorCode::Conflict
    );
    let plan = download
        .prepare(&root.0, &root.directory(), "later.bin", &digest, &cancel)
        .unwrap();
    fs::write(root.0.join("later.bin"), b"external").unwrap();
    assert!(plan.apply(DeletionMode::Permanent).is_err());
    assert_eq!(fs::read(root.0.join("later.bin")).unwrap(), b"external");
}

#[test]
fn preparation_checks_exact_length_hash_and_cancellation() {
    let root = Root::new();
    let cancel = AtomicBool::new(false);
    let download = Download::create(&root.0.join(".runyte"), 3, &cancel).unwrap();
    let directory = root.directory();
    for bytes in [b"ab".as_slice(), b"abcd", b"xyz"] {
        fs::write(download.path(), bytes).unwrap();
        assert_eq!(
            download
                .prepare(
                    &root.0,
                    &directory,
                    "result",
                    &crate::hash::sha256_hex(b"abc"),
                    &cancel
                )
                .unwrap_err()
                .code,
            ErrorCode::Conflict
        );
    }
    fs::write(download.path(), b"abc").unwrap();
    assert_eq!(
        download
            .prepare(&root.0, &directory, "result", "bad", &cancel)
            .unwrap_err()
            .code,
        ErrorCode::InvalidArgument
    );
    cancel.store(true, Ordering::Release);
    assert_eq!(
        download
            .prepare(
                &root.0,
                &directory,
                "result",
                &crate::hash::sha256_hex(b"abc"),
                &cancel
            )
            .unwrap_err()
            .code,
        ErrorCode::Cancelled
    );
    assert!(!root.0.join("result").exists());
}

#[test]
fn creation_enforces_limit_and_cancel_before_creating_runtime_directory() {
    let root = Root::new();
    assert_eq!(
        Download::create(
            &root.0.join(".runyte"),
            MAX_BYTES + 1,
            &AtomicBool::new(false)
        )
        .unwrap_err()
        .code,
        ErrorCode::LimitExceeded
    );
    assert_eq!(
        Download::create(&root.0.join(".runyte"), 0, &AtomicBool::new(true))
            .unwrap_err()
            .code,
        ErrorCode::Cancelled
    );
    assert!(!root.0.join(".runyte").exists());
}

#[test]
fn configured_runtime_root_outside_workspace_retains_private_copy_source() {
    let root = Root::new();
    let runtime = Root::new();
    let cancel = AtomicBool::new(false);
    let download = Download::create(&runtime.0, 3, &cancel).unwrap();
    assert!(
        download
            .path()
            .starts_with(runtime.0.join("cache/plugin-downloads"))
    );
    assert!(!root.0.join(".runyte").exists());
    fs::write(download.path(), b"abc").unwrap();
    let plan = download
        .prepare(
            &root.0,
            &root.directory(),
            "result",
            &crate::hash::sha256_hex(b"abc"),
            &cancel,
        )
        .unwrap();
    assert_eq!(plan.lines(), ["copy downloaded file → result"]);
    drop(download);
    plan.apply(DeletionMode::Permanent).unwrap();
    assert_eq!(fs::read(root.0.join("result")).unwrap(), b"abc");
}

#[cfg(unix)]
#[test]
fn private_storage_rejects_symlink_hardlink_fifo_and_replaced_inode() {
    use std::os::unix::{
        ffi::OsStrExt,
        fs::{MetadataExt, PermissionsExt, symlink},
    };
    let root = Root::new();
    let cancel = AtomicBool::new(false);
    let foreign = root.0.join("foreign");
    fs::write(&foreign, b"abc").unwrap();
    fs::set_permissions(&foreign, fs::Permissions::from_mode(0o644)).unwrap();
    for kind in 0..4 {
        let download = Download::create(&root.0.join(".runyte"), 3, &cancel).unwrap();
        let issued = download.path().to_owned();
        assert_eq!(fs::metadata(&issued).unwrap().mode() & 0o777, 0o600);
        fs::remove_file(&issued).unwrap();
        match kind {
            0 => symlink(&foreign, &issued).unwrap(),
            1 => fs::hard_link(&foreign, &issued).unwrap(),
            2 => {
                let name = std::ffi::CString::new(issued.as_os_str().as_bytes()).unwrap();
                assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
            }
            _ => {
                fs::write(&issued, b"abc").unwrap();
                fs::set_permissions(&issued, fs::Permissions::from_mode(0o644)).unwrap();
            }
        }
        assert_eq!(
            download
                .prepare(
                    &root.0,
                    &root.directory(),
                    "result",
                    &crate::hash::sha256_hex(b"abc"),
                    &cancel
                )
                .unwrap_err()
                .code,
            ErrorCode::Conflict
        );
        if kind == 3 {
            assert_eq!(fs::metadata(&issued).unwrap().mode() & 0o777, 0o644);
        }
        drop(download);
        assert_eq!(fs::read(&foreign).unwrap(), b"abc");
        assert_eq!(fs::metadata(&foreign).unwrap().mode() & 0o777, 0o644);
        fs::remove_file(&issued).unwrap();
    }
}

#[cfg(unix)]
#[test]
fn runtime_directory_symlink_and_workspace_escape_are_refused() {
    let root = Root::new();
    let outside = Root::new();
    std::os::unix::fs::symlink(&outside.0, root.0.join(".runyte")).unwrap();
    assert!(Download::create(&root.0.join(".runyte"), 0, &AtomicBool::new(false)).is_err());
    assert!(!outside.0.join("cache").exists());
    fs::remove_file(root.0.join(".runyte")).unwrap();
    let download = Download::create(&root.0.join(".runyte"), 0, &AtomicBool::new(false)).unwrap();
    let error = download
        .prepare(
            &root.0,
            &root.directory(),
            "../outside",
            &crate::hash::sha256_hex(&[]),
            &AtomicBool::new(false),
        )
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::InvalidArgument);
}
