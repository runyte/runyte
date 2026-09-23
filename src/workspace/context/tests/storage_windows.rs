// SPDX-License-Identifier: MPL-2.0

use super::*;
use crate::test_support::TestRuntimeRoot;
use std::{
    collections::BTreeSet,
    ffi::{OsStr, OsString},
    os::windows::ffi::OsStringExt,
};

fn fixture(name: &str) -> (TestRuntimeRoot, Storage) {
    let root = TestRuntimeRoot::new(name).unwrap();
    std::fs::create_dir(root.join("project")).unwrap();
    let storage = Storage::new(root.join("context")).unwrap();
    (root, storage)
}

#[test]
fn default_location_is_separate_local_app_data_and_override_is_absolute_only() {
    let local = crate::user_paths::system_local_app_data_directory().unwrap();
    let location = select_default_location(None, Some(local.clone())).unwrap();
    assert_eq!(location.root(), local.join("runyte").join("context"));
    assert_eq!(location.anchor, local);

    let fixture = TestRuntimeRoot::new("context-native-location").unwrap();
    let explicit = fixture.join("private-context");
    assert_eq!(
        select_default_location(Some(explicit.clone().into_os_string()), None)
            .unwrap()
            .root(),
        explicit
    );
    assert_eq!(
        select_default_location(Some(OsString::from("relative")), None),
        None
    );
    assert_eq!(
        select_default_location(
            Some(fixture.join("missing/private-context").into_os_string()),
            None
        ),
        None
    );
}

#[test]
fn native_random_identity_and_fail_fast_lock_are_bounded() {
    let (_root, storage) = fixture("context-native-lock");
    let first = random_token().unwrap();
    let second = random_token().unwrap();
    assert!(valid_hex(&first));
    assert!(valid_hex(&second));
    assert_ne!(first, second);

    let held = storage.lock().unwrap();
    assert_eq!(
        storage.identity("agent").unwrap_err().kind(),
        io::ErrorKind::WouldBlock
    );
    drop(held);
    let identity = storage.identity("agent").unwrap();
    assert_eq!(
        storage.identity("agent").unwrap().credential(),
        identity.credential()
    );
}

#[test]
fn grants_round_trip_through_native_private_storage() {
    let (root, storage) = fixture("context-native-records");
    let project = root.join("project").canonicalize().unwrap();
    let identity = storage.identity("agent").unwrap();
    let scopes = BTreeSet::from([Scope::TerminalRead, Scope::EditorContextRead]);
    storage.grant(&project, &identity, scopes.clone()).unwrap();
    assert_eq!(storage.scopes(&project, &identity).unwrap(), scopes);
    storage.revoke(&project, &identity).unwrap();
    assert!(storage.scopes(&project, &identity).unwrap().is_empty());
}

#[test]
fn grant_path_identity_preserves_every_native_path_unit() {
    let root = PathBuf::from(OsString::from_wide(&[
        b'C' as u16,
        b':' as u16,
        b'\\' as u16,
        b'r' as u16,
        0xd800,
    ]));
    let replacement = PathBuf::from(OsString::from_wide(&[
        b'C' as u16,
        b':' as u16,
        b'\\' as u16,
        b'r' as u16,
        0xfffd,
    ]));
    assert_ne!(path_bytes(&root), path_bytes(&replacement));
    assert_eq!(&path_bytes(&root)[8..], &0xd800_u16.to_le_bytes());
    assert_ne!(
        grant_file(&path_bytes(&root), "identity"),
        grant_file(&path_bytes(&replacement), "identity")
    );
}

#[test]
fn durable_storage_creation_reopens_the_established_hierarchy() {
    let root = TestRuntimeRoot::new("context-native-durable").unwrap();
    let path = root.join("parent/context");
    let location = StorageLocation::anchored(path.clone(), root.path().to_owned()).unwrap();
    // Model a child left by an interrupted first attempt. The captured anchor
    // remains the fixture root rather than being rediscovered as `parent`.
    std::fs::create_dir(root.join("parent")).unwrap();
    let first = Storage::open_location(location.clone()).unwrap();
    first.identity("agent").unwrap();
    drop(first);
    let reopened = Storage::open_location(location).unwrap();
    assert!(reopened.load_identity("agent").unwrap().is_some());
}

#[test]
fn explicit_root_requires_its_immediate_parent_before_mutation() {
    let root = TestRuntimeRoot::new("context-native-override-parent").unwrap();
    let parent = root.join("missing");
    let path = parent.join("context");
    assert_eq!(
        Storage::new(path.clone()).unwrap_err().kind(),
        io::ErrorKind::InvalidData
    );
    assert!(!parent.exists());
    std::fs::create_dir(&parent).unwrap();
    Storage::new(path).unwrap();
}

#[test]
fn inventory_and_writes_stay_relative_to_the_pinned_directory() {
    let root = TestRuntimeRoot::new("context-native-pinned").unwrap();
    let path = root.join("context");
    let storage = Storage::new(path.clone()).unwrap();
    std::fs::rename(&path, root.join("retired")).unwrap();
    let replacement = Storage::new(path).unwrap();

    storage.identity("original").unwrap();
    assert_eq!(storage.identities().unwrap()[0].name(), "original");
    assert!(replacement.identities().unwrap().is_empty());
    assert!(root.join("retired/identity-original.json").exists());
    assert!(!root.join("context/identity-original.json").exists());
}

#[test]
fn native_inventory_limit_counts_every_pinned_entry() {
    let (_root, storage) = fixture("context-native-inventory");
    for index in 0..=DISCOVERY_LIMIT {
        drop(
            storage
                .directory
                .create_new(OsStr::new(&format!("unrelated-{index}")))
                .unwrap(),
        );
    }
    assert_eq!(
        storage.identities().unwrap_err().kind(),
        io::ErrorKind::InvalidData
    );
}
