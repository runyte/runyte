// SPDX-License-Identifier: MPL-2.0

#![cfg(unix)]

use runyte::{
    log::{Level, Logger, MAX_LOG_BYTES, Role, Settings, Sink, previous_path},
    lsp_trust::TrustStore,
    pasted_image::{self, ImageFormat},
    test_support::TestRuntimeRoot,
};
use std::{
    fs,
    os::unix::fs::{PermissionsExt, symlink},
    path::PathBuf,
    time::Duration,
};

fn sandbox() -> TestRuntimeRoot {
    // Parallel tests can observe the same clock tick. The shared allocator
    // claims a directory exclusively and retains ownership through cleanup.
    TestRuntimeRoot::new("security").unwrap()
}
const PNG: &[u8] = b"\x89PNG\r\n\x1a\nsafe regression fixture";

#[test]
fn image_storage_refuses_links_and_corrupt_existing_content() {
    let sandbox = sandbox();
    let state = sandbox.join("state");
    let directory = pasted_image::cache_directory(&state);
    fs::create_dir_all(&directory).unwrap();
    let victim = sandbox.join("victim");
    fs::write(&victim, "original").unwrap();
    let name = pasted_image::file_name(PNG, ImageFormat::Png);
    // The former predictable temporary leaf must never be opened for writing.
    symlink(
        &victim,
        directory.join(format!(".{name}.{}", std::process::id())),
    )
    .unwrap();
    let path = pasted_image::store(&state, PNG, ImageFormat::Png).unwrap();
    assert_eq!(fs::read(&victim).unwrap(), b"original");
    assert_eq!(fs::read(&path).unwrap(), PNG);
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert_eq!(
        fs::metadata(&directory).unwrap().permissions().mode() & 0o777,
        0o700
    );
    fs::remove_file(&path).unwrap();
    symlink(&victim, &path).unwrap();
    assert!(pasted_image::store(&state, PNG, ImageFormat::Png).is_err());
    fs::remove_file(&path).unwrap();
    fs::hard_link(&victim, &path).unwrap();
    assert!(pasted_image::store(&state, PNG, ImageFormat::Png).is_err());
    fs::remove_file(&path).unwrap();
    fs::write(&path, b"wrong content").unwrap();
    assert!(pasted_image::store(&state, PNG, ImageFormat::Png).is_err());
    assert_eq!(fs::read(&victim).unwrap(), b"original");
}

#[test]
fn image_and_log_storage_refuse_symlinked_parents() {
    let sandbox = sandbox();
    let outside = sandbox.join("outside");
    fs::create_dir(&outside).unwrap();
    let state = sandbox.join("state");
    symlink(&outside, &state).unwrap();
    assert!(pasted_image::store(&state, PNG, ImageFormat::Png).is_err());
    assert!(
        Logger::start(
            Settings::new(Level::Warn, Role::Host),
            Sink::file(state.join("host.log"))
        )
        .is_err()
    );
    assert_eq!(fs::read_dir(outside).unwrap().count(), 0);
}

#[test]
fn logs_refuse_symlinks_hardlinks_and_fifos_without_modifying_targets() {
    use std::{ffi::CString, os::unix::ffi::OsStrExt};
    let sandbox = sandbox();
    let victim = sandbox.join("victim");
    fs::write(&victim, b"untouched").unwrap();
    let path = sandbox.join("host.log");
    for hard in [false, true] {
        if hard {
            fs::hard_link(&victim, &path).unwrap();
        } else {
            symlink(&victim, &path).unwrap();
        }
        assert!(Logger::start(Settings::new(Level::Warn, Role::Host), Sink::file(&path)).is_err());
        assert_eq!(fs::read(&victim).unwrap(), b"untouched");
        fs::remove_file(&path).unwrap();
    }
    let name = CString::new(path.as_os_str().as_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
    assert!(Logger::start(Settings::new(Level::Warn, Role::Host), Sink::file(&path)).is_err());
}

#[test]
fn log_rotation_uses_held_directory_and_file_and_private_atomic_backup() {
    let sandbox = sandbox();
    let state = sandbox.join("state");
    fs::create_dir(&state).unwrap();
    let path = state.join("host.log");
    let logger = Logger::start(
        Settings::new(Level::Warn, Role::Host),
        Sink::exclusive_file(&path),
    )
    .unwrap();
    let moved = sandbox.join("moved");
    fs::rename(&state, &moved).unwrap();
    let outside = sandbox.join("outside");
    fs::create_dir(&outside).unwrap();
    symlink(&outside, &state).unwrap();
    let victim = sandbox.join("victim");
    fs::write(&victim, "original").unwrap();
    symlink(&victim, previous_path(&moved.join("host.log"))).unwrap();
    for _ in 0..5 {
        logger.emit(
            Level::Warn,
            "security",
            &"x".repeat(MAX_LOG_BYTES as usize / 4),
        );
    }
    logger.flush(Duration::from_secs(5));
    assert!(logger.failure().is_none());
    drop(logger);
    assert_eq!(fs::read(&victim).unwrap(), b"original");
    assert_eq!(fs::read_dir(outside).unwrap().count(), 0);
    for name in ["host.log", "host.log.1"] {
        let metadata = fs::symlink_metadata(moved.join(name)).unwrap();
        assert!(metadata.is_file());
        assert_eq!(metadata.permissions().mode() & 0o777, 0o600);
        assert!(metadata.len() <= MAX_LOG_BYTES);
    }
}

#[test]
fn lsp_decisions_are_private_exact_workspace_records() {
    let sandbox = sandbox();
    let project = sandbox.join("project");
    fs::create_dir(&project).unwrap();
    let nested = project.join("nested");
    fs::create_dir(&nested).unwrap();
    let storage = sandbox.join("user/trust");
    let store = TrustStore::new(Some(storage.clone()), &project).unwrap();
    assert_eq!(store.load().unwrap(), None);
    assert!(
        !storage.exists(),
        "reading an unknown decision does not create storage"
    );
    store.save(true).unwrap();
    assert_eq!(
        TrustStore::new(Some(storage.clone()), &project)
            .unwrap()
            .load()
            .unwrap(),
        Some(true)
    );
    assert_eq!(
        TrustStore::new(Some(storage.clone()), &nested)
            .unwrap()
            .load()
            .unwrap(),
        None
    );
    let alias = sandbox.join("alias");
    symlink(&project, &alias).unwrap();
    assert_eq!(
        TrustStore::new(Some(storage.clone()), &alias)
            .unwrap()
            .load()
            .unwrap(),
        Some(true)
    );
    store.forget().unwrap();
    assert_eq!(store.load().unwrap(), None);
    store.save(false).unwrap();
    assert_eq!(store.load().unwrap(), Some(false));
    assert_eq!(
        fs::metadata(&storage).unwrap().permissions().mode() & 0o777,
        0o700
    );
    let record = fs::read_dir(&storage)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    assert_eq!(
        fs::metadata(&record).unwrap().permissions().mode() & 0o777,
        0o600
    );
    fs::write(&record, b"invalid").unwrap();
    assert!(store.load().is_err());
    fs::remove_file(&record).unwrap();
    let victim = sandbox.join("victim");
    fs::write(&victim, b"untouched").unwrap();
    symlink(&victim, &record).unwrap();
    assert!(store.load().is_err());
    store.save(false).unwrap();
    assert_eq!(store.load().unwrap(), Some(false));
    assert_eq!(fs::read(&victim).unwrap(), b"untouched");
    assert!(TrustStore::new(Some(project.join("trust")), &project).is_err());
    assert!(TrustStore::new(Some(PathBuf::from("relative")), &project).is_err());
    assert!(TrustStore::new(None, &project).unwrap().save(true).is_err());
}
