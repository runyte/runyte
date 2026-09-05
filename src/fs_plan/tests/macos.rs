// SPDX-License-Identifier: MPL-2.0

use super::*;
use std::{ffi::CStr, fs::File, os::fd::AsRawFd, os::unix::fs::PermissionsExt, process::Command};

fn set_attribute(file: &File, name: &CStr, bytes: &[u8]) {
    // SAFETY: the descriptor, C string and byte slice remain valid for the
    // call, and the supplied size is the slice's actual allocation length.
    let result = unsafe {
        libc::fsetxattr(
            file.as_raw_fd(),
            name.as_ptr(),
            bytes.as_ptr().cast(),
            bytes.len(),
            0,
            0,
        )
    };
    assert_eq!(result, 0, "{}", io::Error::last_os_error());
}

fn attribute(file: &File, name: &CStr) -> Vec<u8> {
    // SAFETY: a null buffer with length zero queries the attribute size.
    let size = unsafe {
        libc::fgetxattr(
            file.as_raw_fd(),
            name.as_ptr(),
            std::ptr::null_mut(),
            0,
            0,
            0,
        )
    };
    assert!(size >= 0, "{}", io::Error::last_os_error());
    let mut value = vec![0; size as usize];
    // SAFETY: the allocated slice is writable for exactly the supplied length.
    let read = unsafe {
        libc::fgetxattr(
            file.as_raw_fd(),
            name.as_ptr(),
            value.as_mut_ptr().cast(),
            value.len(),
            0,
            0,
        )
    };
    assert_eq!(read, size, "{}", io::Error::last_os_error());
    value
}

fn decorate(path: &Path, label: &str) {
    fs::write(path, format!("data {label}")).unwrap();
    let mode = if label == "original" { 0o640 } else { 0o600 };
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
    let file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .unwrap();
    set_attribute(
        &file,
        c"com.runyte.copy-test",
        format!("attribute {label}").as_bytes(),
    );
    set_attribute(
        &file,
        c"com.apple.ResourceFork",
        format!("resource fork {label}").as_bytes(),
    );
    // Run only the system utility, never the file this test just wrote.
    let rights = if label == "original" {
        "everyone allow read,readattr,readextattr,readsecurity"
    } else {
        "everyone allow read,readattr"
    };
    let result = Command::new("/bin/chmod")
        .args(["+a", rights])
        .arg(path)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
}

#[derive(Debug, PartialEq, Eq)]
struct Contents {
    data: Vec<u8>,
    attribute: Vec<u8>,
    resource_fork: Vec<u8>,
    mode: u32,
    acl: Vec<String>,
}

fn contents(path: &Path) -> Contents {
    let file = File::open(path).unwrap();
    let listing = Command::new("/bin/ls")
        .arg("-lde")
        .arg(path)
        .output()
        .unwrap();
    assert!(
        listing.status.success(),
        "{}",
        String::from_utf8_lossy(&listing.stderr)
    );
    // The first row contains the differing filename. Subsequent rows contain
    // the ACL entries in order; require a real ACL so the assertion is useful.
    let acl = String::from_utf8(listing.stdout)
        .unwrap()
        .lines()
        .skip(1)
        .map(str::to_owned)
        .collect::<Vec<_>>();
    assert!(!acl.is_empty(), "fixture must carry an extended ACL");
    Contents {
        data: fs::read(path).unwrap(),
        attribute: attribute(&file, c"com.runyte.copy-test"),
        resource_fork: attribute(&file, c"com.apple.ResourceFork"),
        mode: file.metadata().unwrap().permissions().mode() & 0o777,
        acl,
    }
}

fn plan_with_metadata(dir: &TempDir, nested: bool) -> (FsPlan, PathBuf) {
    let source = if nested {
        fs::create_dir(dir.join("source")).unwrap();
        dir.join("source/child")
    } else {
        dir.join("source")
    };
    decorate(&source, "original");
    // Capture the confirmed identity only after adding the metadata.
    let snapshot = DirectorySnapshot::read(&dir.0).unwrap();
    let entry = &snapshot.entries()[0];
    let desired = vec![
        DesiredEntry::existing(entry, "source"),
        DesiredEntry::existing(entry, "destination"),
    ];
    (
        FsPlan::build(dir.0.clone(), snapshot, desired).unwrap(),
        source,
    )
}

#[test]
fn copies_preserve_resource_forks_xattrs_and_acls() {
    for nested in [false, true] {
        let dir = TempDir::new();
        let (plan, source) = plan_with_metadata(&dir, nested);
        let expected = contents(&source);
        let report = plan
            .apply_with_trash(DeletionMode::Permanent, &NoTrash)
            .unwrap();
        assert!(report.recovery.is_empty());
        let target = dir.join(if nested {
            "destination/child"
        } else {
            "destination"
        });
        assert_eq!(contents(&target), expected);
        assert_eq!(contents(&source), expected);
    }
}

#[test]
fn publication_collision_preserves_source_and_competing_metadata() {
    for nested in [false, true] {
        let dir = TempDir::new();
        let (plan, source) = plan_with_metadata(&dir, nested);
        let expected = contents(&source);
        let staged_expected = contents(&source);
        let competing_expected = Rc::new(RefCell::new(None));
        let observed = Rc::clone(&competing_expected);
        let error = apply(&plan, move |step, staged, target| {
            if step == IoStep::Publish {
                let staged = if nested {
                    staged.join("child")
                } else {
                    staged.to_path_buf()
                };
                assert_eq!(contents(&staged), staged_expected);
                if nested {
                    fs::create_dir(target)?;
                }
                let competing = if nested {
                    target.join("child")
                } else {
                    target.to_path_buf()
                };
                decorate(&competing, "competing");
                *observed.borrow_mut() = Some(contents(&competing));
            }
            Ok(())
        })
        .unwrap_err();
        assert!(error.report.applied.is_empty());
        assert!(error.report.recovery.is_empty());
        assert_eq!(contents(&source), expected);
        let target = dir.join(if nested {
            "destination/child"
        } else {
            "destination"
        });
        let competing = contents(&target);
        assert_eq!(competing.data, b"data competing");
        assert_eq!(competing.attribute, b"attribute competing");
        assert_eq!(competing.resource_fork, b"resource fork competing");
        assert_ne!(competing.acl, expected.acl);
        assert_ne!(competing.mode, expected.mode);
        assert_eq!(competing, competing_expected.borrow_mut().take().unwrap());
        assert_eq!(fs::read_dir(&dir.0).unwrap().count(), 2);
    }
}

#[test]
fn native_copy_errors_do_not_report_success() {
    let dir = TempDir::new();
    decorate(&dir.join("source"), "original");
    decorate(&dir.join("target"), "unchanged");
    let expected = contents(&dir.join("source"));
    let mut source = File::open(dir.join("source")).unwrap();
    let mut target = File::open(dir.join("target")).unwrap();
    assert!(platform::copy_file(&mut source, &mut target).is_err());
    assert_eq!(contents(&dir.join("source")), expected);
}
