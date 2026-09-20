// SPDX-License-Identifier: MPL-2.0

use super::Directory;
use crate::test_support::TestRuntimeRoot;
use std::{
    ffi::OsStr,
    fs,
    io::Write,
    os::unix::fs::{MetadataExt, PermissionsExt, symlink},
    sync::Barrier,
};

#[test]
fn concurrent_append_creation_keeps_one_inode_and_every_write() {
    let root = TestRuntimeRoot::new("append-race").unwrap();
    let directory = Directory::open(&root, true).unwrap();
    for round in 0..32 {
        let name = format!("append-{round}");
        let barrier = Barrier::new(8);
        let identities = std::thread::scope(|scope| {
            let writers: Vec<_> = (0..8)
                .map(|index| {
                    let (directory, name, barrier) = (&directory, &name, &barrier);
                    scope.spawn(move || {
                        barrier.wait();
                        let mut file = directory.append(OsStr::new(name)).unwrap();
                        let metadata = file.metadata().unwrap();
                        file.write_all(&[b'0' + index]).unwrap();
                        (metadata.dev(), metadata.ino())
                    })
                })
                .collect();
            writers
                .into_iter()
                .map(|writer| writer.join().unwrap())
                .collect::<Vec<_>>()
        });
        assert!(identities.iter().all(|identity| *identity == identities[0]));
        let mut bytes = fs::read(root.join(&name)).unwrap();
        bytes.sort_unstable();
        assert_eq!(bytes, b"01234567");
        let metadata = fs::metadata(root.join(&name)).unwrap();
        assert_eq!((metadata.dev(), metadata.ino()), identities[0]);
        assert_eq!(metadata.mode() & 0o777, 0o600);
        directory
            .append(OsStr::new(&name))
            .unwrap()
            .write_all(b"tail")
            .unwrap();
        assert_eq!(&fs::read(root.join(&name)).unwrap()[8..], b"tail");
    }
}

#[test]
fn append_existing_rejects_links_and_preserves_unadmitted_files() {
    let root = TestRuntimeRoot::new("append-links").unwrap();
    let directory = Directory::open(&root, true).unwrap();
    let target = root.join("target");
    fs::write(&target, b"unchanged").unwrap();
    fs::set_permissions(&target, fs::Permissions::from_mode(0o640)).unwrap();
    symlink(&target, root.join("symlink")).unwrap();
    assert!(directory.append(OsStr::new("symlink")).is_err());
    assert!(root.join("symlink").is_symlink());
    fs::hard_link(&target, root.join("hardlink")).unwrap();
    assert!(directory.append(OsStr::new("hardlink")).is_err());
    assert!(root.join("hardlink").exists());
    assert_eq!(fs::read(&target).unwrap(), b"unchanged");
    assert_eq!(fs::metadata(&target).unwrap().mode() & 0o777, 0o640);
}
