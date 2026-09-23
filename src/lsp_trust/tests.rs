// SPDX-License-Identifier: MPL-2.0

use super::*;
use crate::test_support::TestRuntimeRoot;

#[test]
#[cfg(unix)]
fn home_exception_rejects_project_cache_overrides_and_symlinks() {
    let root = TestRuntimeRoot::new("trust-home").unwrap();
    let home = root.create_private_dir("home").unwrap();
    let project = home.join("project");
    std::fs::create_dir(&project).unwrap();
    let standard = home.join(TrustStore::HOME_CACHE);
    assert!(TrustStore::new_with_home(Some(standard.clone()), &home, Some(&home)).is_ok());
    assert!(TrustStore::new_with_home(Some(standard.clone()), &home, None).is_err());
    assert!(
        TrustStore::new_with_home(Some(home.join("custom-cache")), &home, Some(&home)).is_err()
    );
    assert!(
        TrustStore::new_with_home(
            Some(project.join(TrustStore::HOME_CACHE)),
            &project,
            Some(&home)
        )
        .is_err()
    );
    assert!(TrustStore::new_with_home(Some(standard.clone()), &root, Some(&home)).is_err());
    assert!(TrustStore::new_with_home(Some(standard.clone()), &project, Some(&home)).is_ok());

    let alias = root.join("home-alias");
    std::os::unix::fs::symlink(&home, &alias).unwrap();
    let store = TrustStore::new_with_home(Some(standard.clone()), &alias, Some(&home)).unwrap();
    store.save(true).unwrap();
    assert_eq!(store.load().unwrap(), Some(true));

    // Recognizing the standard location must not bypass secure opening.
    std::fs::remove_dir_all(&standard).unwrap();
    let outside = root.create_private_dir("outside").unwrap();
    std::os::unix::fs::symlink(&outside, &standard).unwrap();
    assert!(store.load().is_err());
    assert!(store.save(true).is_err());
    assert!(std::fs::read_dir(outside).unwrap().next().is_none());
}

#[test]
fn remembered_answers_and_forgetting_use_only_the_injected_store() {
    let root = TestRuntimeRoot::new("trust-decisions").unwrap();
    let project = root.create_private_dir("project").unwrap();
    let storage = root.join("cache/lsp-trust");
    let store =
        TrustStore::new_with_account_paths(Some(storage.clone()), &project, None, None).unwrap();
    assert_eq!(store.load().unwrap(), None);
    store.forget().unwrap();
    assert!(!storage.exists());
    store.save(true).unwrap();
    assert_eq!(store.load().unwrap(), Some(true));
    store.save(false).unwrap();
    assert_eq!(store.load().unwrap(), Some(false));
    store.forget().unwrap();
    assert_eq!(store.load().unwrap(), None);
    let temporary = TrustStore::new_with_account_paths(None, &project, None, None).unwrap();
    assert!(!temporary.can_remember());
    assert_eq!(temporary.load().unwrap(), None);
    temporary.forget().unwrap();
    assert!(temporary.save(true).is_err());
}

#[test]
fn records_for_another_workspace_or_malformed_records_fail_closed() {
    let root = TestRuntimeRoot::new("trust-records").unwrap();
    let project = root.create_private_dir("project").unwrap();
    let storage = root.create_private_dir("cache").unwrap();
    let store =
        TrustStore::new_with_account_paths(Some(storage.clone()), &project, None, None).unwrap();
    let directory = crate::private_storage::Directory::open(&storage, true).unwrap();
    let other = serde_json::to_vec(&Decision {
        project: b"another workspace".to_vec(),
        allowed: true,
    })
    .unwrap();
    for bytes in [other, b"not JSON".to_vec(), vec![b' '; 64 * 1024 + 1]] {
        directory.atomic_write(store.name.as_ref(), &bytes).unwrap();
        assert!(store.load().is_err());
    }
}

#[cfg(windows)]
#[test]
fn canonical_native_identity_matches_ordinary_and_verbatim_spelling() {
    let root = TestRuntimeRoot::new("trust-spelling").unwrap();
    let project = root.create_private_dir("project-é-😀").unwrap();
    let ordinary = crate::windows_fs::ordinary_working_directory(&project).unwrap();
    let extended = project.canonicalize().unwrap();
    assert_ne!(ordinary, extended);
    let storage = root.join("cache/lsp-trust");
    let first =
        TrustStore::new_with_account_paths(Some(storage.clone()), &ordinary, None, None).unwrap();
    let second = TrustStore::new_with_account_paths(Some(storage), &extended, None, None).unwrap();
    assert_eq!(first.project, second.project);
    assert_eq!(first.name, second.name);
    first.save(true).unwrap();
    assert_eq!(second.load().unwrap(), Some(true));
    second.forget().unwrap();
    assert_eq!(first.load().unwrap(), None);
}

#[cfg(windows)]
#[test]
fn native_trust_identity_preserves_unpaired_utf16_units() {
    use std::{ffi::OsString, os::windows::ffi::OsStringExt};
    let make = |unit| {
        PathBuf::from(OsString::from_wide(&[
            b'C' as u16,
            b':' as u16,
            b'\\' as u16,
            unit,
        ]))
    };
    let identities: Vec<_> = [0xD800, 0xD801, 0xFFFD]
        .into_iter()
        .map(|unit| project_bytes(&make(unit)))
        .collect();
    assert_eq!(identities[0], vec![b'C', 0, b':', 0, b'\\', 0, 0, 0xD8]);
    for (index, identity) in identities.iter().enumerate() {
        for other in &identities[index + 1..] {
            assert_ne!(identity, other);
            assert_ne!(
                crate::hash::sha256_hex(identity),
                crate::hash::sha256_hex(other)
            );
        }
    }
}

#[cfg(windows)]
#[test]
fn deep_nonexistent_native_cache_overrides_cannot_overlap_the_workspace() {
    let root = TestRuntimeRoot::new("trust-overlap").unwrap();
    let project = root.create_private_dir("project").unwrap();
    let ordinary = crate::windows_fs::ordinary_working_directory(&project).unwrap();
    let storage = ordinary.join("missing/deep/cache/lsp-trust");
    assert!(
        TrustStore::new_with_account_paths(Some(storage.clone()), &project, None, None).is_err()
    );
    assert!(!storage.exists());
    assert!(
        TrustStore::new_with_account_paths(Some(root.path().to_path_buf()), &project, None, None)
            .is_err()
    );
}

#[cfg(windows)]
#[test]
fn native_home_exception_uses_the_exact_independently_resolved_standard_cache() {
    let root = TestRuntimeRoot::new("trust-account").unwrap();
    let home = root.create_private_dir("home").unwrap();
    let project = home.join("project");
    std::fs::create_dir(&project).unwrap();
    // A redirected account folder inside home need not be AppData/Local.
    let standard = home.join("redirected-local-data/runyte/cache/lsp-trust");
    let ordinary_standard = crate::windows_fs::ordinary_working_directory(&home)
        .unwrap()
        .join("redirected-local-data/runyte/cache/lsp-trust");
    let make = |directory, project: &Path, account_home, standard| {
        TrustStore::new_with_account_paths(Some(directory), project, account_home, standard)
    };
    let store = make(
        ordinary_standard,
        &home,
        Some(home.as_path()),
        Some(standard.as_path()),
    )
    .unwrap();
    store.save(true).unwrap();
    assert_eq!(store.load().unwrap(), Some(true));
    assert!(make(standard.clone(), &home, None, Some(standard.as_path())).is_err());
    assert!(make(standard.clone(), &home, Some(home.as_path()), None).is_err());
    assert!(
        make(
            home.join("custom-cache"),
            &home,
            Some(home.as_path()),
            Some(standard.as_path())
        )
        .is_err()
    );
    assert!(
        make(
            project.join("cache"),
            &project,
            Some(home.as_path()),
            Some(standard.as_path())
        )
        .is_err()
    );
    assert!(
        make(
            standard.clone(),
            root.path(),
            Some(home.as_path()),
            Some(standard.as_path())
        )
        .is_err()
    );
    let redirected = root.join("outside-profile/runyte/cache/lsp-trust");
    let outside = make(
        redirected.clone(),
        &home,
        Some(home.as_path()),
        Some(redirected.as_path()),
    )
    .unwrap();
    outside.save(false).unwrap();
    assert_eq!(outside.load().unwrap(), Some(false));
}

#[cfg(windows)]
#[test]
fn native_home_exception_does_not_bypass_junction_admission() {
    use std::os::windows::{ffi::OsStrExt, fs::OpenOptionsExt, io::AsRawHandle};
    use windows_sys::Win32::{
        Storage::FileSystem::{FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT},
        System::IO::DeviceIoControl,
    };
    let root = TestRuntimeRoot::new("trust-junction").unwrap();
    let home = root.create_private_dir("home").unwrap();
    let outside = root.create_private_dir("outside").unwrap();
    let standard = home.join("standard-trust");
    std::fs::create_dir(&standard).unwrap();
    let handle = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(&standard)
        .unwrap();
    let ordinary = crate::windows_fs::ordinary_working_directory(&outside).unwrap();
    let substitute: Vec<u16> = std::ffi::OsStr::new(r"\??\")
        .encode_wide()
        .chain(ordinary.as_os_str().encode_wide())
        .collect();
    // Native mount-point fixture, requiring neither a shell nor symlink privilege.
    let mut data = Vec::new();
    data.extend_from_slice(&0xA0000003u32.to_le_bytes()); // IO_REPARSE_TAG_MOUNT_POINT
    data.extend_from_slice(&((8 + (substitute.len() + 2) * 2) as u16).to_le_bytes());
    data.extend_from_slice(&0u16.to_le_bytes());
    data.extend_from_slice(&0u16.to_le_bytes());
    data.extend_from_slice(&((substitute.len() * 2) as u16).to_le_bytes());
    data.extend_from_slice(&(((substitute.len() + 1) * 2) as u16).to_le_bytes());
    data.extend_from_slice(&0u16.to_le_bytes());
    for unit in substitute {
        data.extend_from_slice(&unit.to_le_bytes());
    }
    data.extend_from_slice(&[0; 4]);
    let mut returned = 0;
    // SAFETY: the live directory handle and byte buffer have the mount-point
    // reparse layout; this synchronous operation has no output buffer.
    assert_ne!(
        unsafe {
            DeviceIoControl(
                handle.as_raw_handle(),
                0x000900A4,
                data.as_ptr().cast(),
                data.len() as u32,
                std::ptr::null_mut(),
                0,
                &mut returned,
                std::ptr::null_mut(),
            )
        },
        0,
        "{}",
        io::Error::last_os_error()
    );
    drop(handle);
    let store = TrustStore::new_with_account_paths(
        Some(standard.clone()),
        &home,
        Some(&home),
        Some(&standard),
    )
    .unwrap();
    assert!(store.load().is_err());
    assert!(store.save(true).is_err());
    assert!(store.forget().is_err());
    assert!(std::fs::read_dir(&outside).unwrap().next().is_none());
    std::fs::remove_dir(standard).unwrap();
}
