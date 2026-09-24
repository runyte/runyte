// SPDX-License-Identifier: MPL-2.0

use super::*;
use crate::test_support::TestRuntimeRoot;
use std::{fs, sync::mpsc, thread};

fn location(root: &Path, registries: RegistrySet) -> EndpointLocation {
    let project = root.join("project");
    fs::create_dir_all(&project).unwrap();
    EndpointLocation::new(&project, root.join("runtime-host"), registries).unwrap()
}

fn fixture(label: &str) -> (TestRuntimeRoot, EndpointLocation, NameStore) {
    let root = TestRuntimeRoot::new(label).unwrap();
    let location = location(
        root.path(),
        RegistrySet::open_fixture(&[root.join("registry")]).unwrap(),
    );
    let names = NameStore::open(&root.join("configured-state")).unwrap();
    (root, location, names)
}

fn publication(location: &EndpointLocation) -> Publication {
    location
        .prepare(Some("old".into()))
        .unwrap()
        .publish()
        .unwrap()
}

fn assert_published(publication: &Publication, expected: &str) {
    assert_eq!(publication.metadata.name.as_deref(), Some(expected));
    for issued in &publication.issued {
        issued.verify_path().unwrap();
        let metadata = if issued.registry {
            RegistryRecord::from_json(&issued.bytes).unwrap().host
        } else {
            EndpointMetadata::from_json(&issued.bytes).unwrap()
        };
        assert_eq!(metadata, publication.metadata);
    }
}

fn assert_no_staging(directory: &Directory) {
    let (entries, truncated) = directory.entries(4096).unwrap();
    assert!(!truncated);
    assert!(
        !entries
            .iter()
            .any(|name| name.to_string_lossy().starts_with(".runyte-transaction-")),
        "remaining entries: {entries:?}"
    );
}

#[test]
fn live_rename_updates_owned_files_keeps_old_readers_and_persists_in_configured_state() {
    let (_root, location, names) = fixture("names-live");
    let mut publication = publication(&location);
    let before = publication.metadata.clone();
    let mut old_readers = publication
        .issued
        .iter()
        .map(|issued| {
            (
                issued
                    .directory
                    .open_read(OsStr::new(&issued.name))
                    .unwrap(),
                issued.bytes.clone(),
            )
        })
        .collect::<Vec<_>>();
    publication.rename(&names, "new").unwrap();
    assert_published(&publication, "new");
    assert_eq!(publication.metadata.process, before.process);
    assert_eq!(publication.metadata.address, before.address);
    assert_eq!(publication.metadata.incarnation, before.incarnation);
    assert!(!publication.rename_recovery_pending());
    assert_eq!(names.load(&before.id).unwrap().as_deref(), Some("new"));
    assert!(!location.directory.join("host-names").exists());
    for (file, expected) in &mut old_readers {
        let mut actual = Vec::new();
        file.read_to_end(&mut actual).unwrap();
        assert_eq!(&actual, expected);
    }
    for issued in &publication.issued {
        assert_no_staging(&issued.directory);
    }
    publication.cleanup().unwrap();
    assert!(!location.ready_record().exists());
    assert!(location.registries.read(&before.id).unwrap().is_empty());
    assert_eq!(names.load(&before.id).unwrap().as_deref(), Some("new"));
    assert_no_staging(&names.directory);
}

#[test]
fn stored_names_preserve_explicit_choice_take_lock_and_reject_bad_records() {
    let (_root, location, names) = fixture("names-store");
    let id = crate::workspace::workspace_id(&location.project);
    assert_eq!(names.load(&id).unwrap(), None);
    assert_eq!(
        names
            .store_if_absent(&location.registries, &id, "chosen")
            .unwrap(),
        "chosen"
    );
    assert_eq!(
        names
            .store_if_absent(&location.registries, &id, "automatic")
            .unwrap(),
        "chosen"
    );
    let prepared = location.prepare_named(&names, None).unwrap();
    assert_eq!(prepared.metadata.name.as_deref(), Some("chosen"));
    drop(prepared);
    let _lock = names.lock().unwrap();
    assert_eq!(
        names.load(&id).unwrap_err().kind(),
        io::ErrorKind::WouldBlock
    );
    drop(_lock);
    for invalid_name in ["", " spaced", "line\nfeed", &"x".repeat(65)] {
        assert!(
            names
                .store_if_absent(&location.registries, &id, invalid_name)
                .is_err()
        );
    }
    for bytes in [
        b"null".to_vec(),
        b"\" spaced\"".to_vec(),
        vec![b'x'; MAX_STORED_NAME_BYTES + 1],
    ] {
        names
            .directory
            .atomic_write(OsStr::new(&format!("{id}.json")), &bytes)
            .unwrap();
        assert!(names.load(&id).is_err());
        assert!(location.prepare_named(&names, None).is_err());
    }
}

#[test]
fn failures_at_every_forward_boundary_roll_back_all_records_and_refresh_owned_handles() {
    for phase in [
        Step::Staged,
        Step::Removing,
        Step::Removed,
        Step::Installed,
        Step::Synced,
    ] {
        for failed_index in 0..3 {
            let (_root, location, names) = fixture("names-forward-failure");
            let mut publication = publication(&location);
            names
                .store_if_absent(
                    &location.registries,
                    &publication.metadata.id,
                    "persisted-before",
                )
                .unwrap();
            let mut injected = false;
            let error = publication
                .rename_with(&names, "new", |index, step, _| {
                    if !injected && index == failed_index && step == phase {
                        injected = true;
                        return Err(io::Error::other("injected forward failure"));
                    }
                    Ok(())
                })
                .unwrap_err();
            assert!(injected);
            assert!(error.to_string().contains("injected forward failure"));
            assert!(
                !publication.rename_recovery_pending(),
                "{phase:?} at {failed_index}: {error}"
            );
            assert_published(&publication, "old");
            assert_eq!(
                names.load(&publication.metadata.id).unwrap().as_deref(),
                Some("persisted-before")
            );
            for issued in &publication.issued {
                assert_no_staging(&issued.directory);
            }
            assert_no_staging(&names.directory);
            publication.cleanup().unwrap();
            assert!(
                location
                    .registries
                    .read(&publication.metadata.id)
                    .unwrap()
                    .is_empty()
            );
        }
    }
}

#[test]
fn failure_restores_absent_stored_name_and_retry_retains_only_bounded_generations() {
    let (_root, location, names) = fixture("names-recovery");
    let mut publication = publication(&location);
    let mut rolled_back = Vec::new();
    let error = publication
        .rename_with(&names, "new", |index, step, _| {
            if step == Step::Installed && index == 2 {
                return Err(io::Error::other("after ready installation"));
            }
            if step == Step::RollingBack {
                rolled_back.push(index);
                if index == 1 {
                    return Err(io::Error::other("registry restoration unavailable"));
                }
            }
            Ok(())
        })
        .unwrap_err();
    assert!(error.to_string().contains("requires recovery"));
    assert_eq!(
        rolled_back,
        vec![2, 1, 0],
        "rollback attempts every record in reverse"
    );
    assert!(publication.rename_recovery_pending());
    assert!(publication.rename(&names, "another").is_err());
    assert!(names.load(&publication.metadata.id).unwrap().is_none());
    let recovery = publication.rename_recovery.as_ref().unwrap();
    assert_eq!(recovery.changes.len(), 3);
    assert!(recovery.changes.iter().all(|change| change.next.is_some()));
    publication.retry_rename_recovery().unwrap();
    assert!(!publication.rename_recovery_pending());
    assert_published(&publication, "old");
    assert!(names.load(&publication.metadata.id).unwrap().is_none());
    publication.rename(&names, "after-recovery").unwrap();
    assert_published(&publication, "after-recovery");
    publication.cleanup().unwrap();
    assert!(
        location
            .registries
            .read(&publication.metadata.id)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn foreign_file_in_install_gap_survives_failure_recovery_and_final_cleanup() {
    let (_root, location, names) = fixture("names-foreign-gap");
    let mut publication = publication(&location);
    let registry = publication.issued[0].directory.clone();
    let registry_name = publication.issued[0].name.clone();
    let registry_path = publication.issued[0].path.clone();
    let error = publication
        .rename_with(&names, "new", |index, step, _| {
            if index == 1 && step == Step::Removed {
                registry
                    .create_new(OsStr::new(&registry_name))?
                    .write_all(b"foreign")?;
            }
            Ok(())
        })
        .unwrap_err();
    assert!(error.to_string().contains("recovery"));
    assert_eq!(fs::read(&registry_path).unwrap(), b"foreign");
    assert!(publication.retry_rename_recovery().is_err());
    assert_eq!(fs::read(&registry_path).unwrap(), b"foreign");
    publication.cleanup().unwrap();
    assert_eq!(fs::read(&registry_path).unwrap(), b"foreign");
    assert!(!location.ready_record().exists());
    assert_no_staging(&registry);
    assert_no_staging(&names.directory);
}

#[test]
fn foreign_stored_name_survives_cleanup_while_all_metadata_generations_are_retired() {
    let (_root, location, names) = fixture("names-foreign-store");
    let mut publication = publication(&location);
    let name = format!("{}.json", publication.metadata.id);
    let error = publication
        .rename_with(&names, "new", |index, step, _| {
            if index == 2 && step == Step::Installed {
                names
                    .directory
                    .atomic_write(OsStr::new(&name), b"\"foreign\"")?;
                return Err(io::Error::other("post-install failure"));
            }
            Ok(())
        })
        .unwrap_err();
    assert!(error.to_string().contains("recovery"));
    assert!(
        publication.cleanup().is_err(),
        "cannot restore absence over a foreign stored name"
    );
    assert_eq!(
        names.load(&publication.metadata.id).unwrap().as_deref(),
        Some("foreign")
    );
    assert!(!location.ready_record().exists());
    assert!(
        location
            .registries
            .read(&publication.metadata.id)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn shared_secondary_reserves_names_but_owner_inventory_does_not_join_isolated_scopes() {
    let root = TestRuntimeRoot::new("names-collision-scope").unwrap();
    let shared = root.join("shared");
    let a = location(
        &root.join("a"),
        RegistrySet::open_fixture(&[root.join("primary-a"), shared.clone()]).unwrap(),
    );
    let b = location(
        &root.join("b"),
        RegistrySet::open_fixture(&[root.join("primary-b"), shared]).unwrap(),
    );
    let names = NameStore::open(&root.join("state")).unwrap();
    let first = a
        .prepare(Some("reserved".into()))
        .unwrap()
        .publish()
        .unwrap();
    assert_eq!(
        b.prepare(Some("reserved".into())).unwrap_err().kind(),
        io::ErrorKind::AlreadyExists
    );
    let mut second = b.prepare(Some("other".into())).unwrap().publish().unwrap();
    assert_eq!(
        second.rename(&names, "reserved").unwrap_err().kind(),
        io::ErrorKind::AlreadyExists
    );
    drop((first, second));
    let inventory = root.join("inventory");
    let c = location(
        &root.join("c"),
        RegistrySet::with_inventory(&[root.join("primary-c")], Some(inventory.clone())).unwrap(),
    );
    let d = location(
        &root.join("d"),
        RegistrySet::with_inventory(&[root.join("primary-d")], Some(inventory)).unwrap(),
    );
    let _third = c.prepare(Some("same".into())).unwrap().publish().unwrap();
    let mut fourth = d.prepare(Some("other".into())).unwrap().publish().unwrap();
    fourth.rename(&names, "same").unwrap();
    assert_published(&fourth, "same");
}

#[test]
fn initial_publication_and_live_rename_share_namespace_guards() {
    let root = TestRuntimeRoot::new("names-initial-race").unwrap();
    let registries = RegistrySet::open_fixture(&[root.join("registry")]).unwrap();
    let a = location(&root.join("a"), registries.clone());
    let b = location(&root.join("b"), registries);
    let names = NameStore::open(&root.join("state")).unwrap();
    let mut first = a.prepare(Some("first".into())).unwrap().publish().unwrap();
    let (ready, received) = mpsc::sync_channel(1);
    let (release, released) = mpsc::sync_channel(1);
    let worker = thread::spawn(move || {
        let prepared = b.prepare(Some("reserved".into())).unwrap();
        ready.send(()).unwrap();
        released
            .recv_timeout(std::time::Duration::from_secs(3))
            .unwrap();
        prepared.publish().unwrap()
    });
    received
        .recv_timeout(std::time::Duration::from_secs(3))
        .unwrap();
    let contested = first.rename(&names, "reserved");
    release.send(()).unwrap();
    let _second = worker.join().unwrap();
    assert_eq!(contested.unwrap_err().kind(), io::ErrorKind::WouldBlock);
    assert_eq!(
        first.rename(&names, "reserved").unwrap_err().kind(),
        io::ErrorKind::AlreadyExists
    );
    assert_published(&first, "first");
}

#[test]
fn malformed_namespace_records_refuse_name_claim_without_mutating_publication() {
    let (_root, location, names) = fixture("names-indeterminate");
    let mut publication = publication(&location);
    let registry = &location.registries.0[0];
    let malformed = format!("{}.json", "f".repeat(32));
    registry
        .directory
        .create_new(OsStr::new(&malformed))
        .unwrap()
        .write_all(b"bad")
        .unwrap();
    assert!(
        publication
            .rename(&names, "new")
            .unwrap_err()
            .to_string()
            .contains("indeterminate")
    );
    assert_published(&publication, "old");
    assert!(names.load(&publication.metadata.id).unwrap().is_none());
    assert!(!publication.rename_recovery_pending());
}

#[test]
fn store_lock_contention_during_cleanup_does_not_leave_new_metadata_published() {
    let (_root, location, names) = fixture("names-cleanup-contention");
    let mut publication = publication(&location);
    publication
        .rename_with(&names, "new", |index, step, _| {
            if (index == 2 && step == Step::Installed) || step == Step::RollingBack {
                return Err(io::Error::other("defer restoration"));
            }
            Ok(())
        })
        .unwrap_err();
    assert!(publication.rename_recovery_pending());
    let lock = names.lock().unwrap();
    assert_eq!(
        publication.cleanup().unwrap_err().kind(),
        io::ErrorKind::WouldBlock
    );
    assert!(!location.ready_record().exists());
    assert!(
        location
            .registries
            .read(&publication.metadata.id)
            .unwrap()
            .is_empty()
    );
    assert!(publication.rename_recovery_pending());
    drop(lock);
    publication.cleanup().unwrap();
    assert!(!publication.rename_recovery_pending());
    assert!(names.load(&publication.metadata.id).unwrap().is_none());
    assert_no_staging(&names.directory);
}

#[test]
fn stored_if_absent_verifies_the_advertised_issued_file_before_success() {
    let (_root, location, names) = fixture("names-store-verify");
    let id = crate::workspace::workspace_id(&location.project);
    let leaf = format!("{id}.json");
    let result = names.store_if_absent_with(&location.registries, &id, "requested", |_| {
        names
            .directory
            .atomic_write(OsStr::new(&leaf), b"\"replacement\"")
    });
    assert!(result.is_err());
    assert_eq!(names.load(&id).unwrap().as_deref(), Some("replacement"));
    assert_no_staging(&names.directory);
}

#[test]
fn retry_keeps_already_installed_restoration_identity_after_a_late_failure() {
    for restored_index in 0..3 {
        let (_root, location, names) = fixture("names-restored-retry");
        let mut publication = publication(&location);
        names
            .store_if_absent(&location.registries, &publication.metadata.id, "explicit")
            .unwrap();
        let mut injected = false;
        publication
            .rename_with(&names, "new", |index, step, _| {
                if index == 2 && step == Step::Installed {
                    return Err(io::Error::other("fail after install"));
                }
                if !injected && index == restored_index && step == Step::Restored {
                    injected = true;
                    return Err(io::Error::other("fail after restoration became visible"));
                }
                Ok(())
            })
            .unwrap_err();
        assert!(injected);
        let change = &publication.rename_recovery.as_ref().unwrap().changes[restored_index];
        let path = change.path.clone();
        let identity = locking::file_key(&change.restore.as_ref().unwrap().file).unwrap();
        let expected = change.old.as_ref().unwrap().bytes.clone();
        assert_eq!(fs::read(&path).unwrap(), expected);
        publication.retry_rename_recovery().unwrap();
        assert!(!publication.rename_recovery_pending());
        assert_eq!(
            locking::file_key(&File::open(&path).unwrap()).unwrap(),
            identity
        );
        assert_eq!(fs::read(&path).unwrap(), expected);
        assert_published(&publication, "old");
        for issued in &publication.issued {
            assert_no_staging(&issued.directory);
        }
        assert_no_staging(&names.directory);
    }
}

#[test]
fn rollback_preserves_a_foreign_ready_replacement_and_still_restores_other_records() {
    let (_root, location, names) = fixture("names-foreign-rollback");
    let mut publication = publication(&location);
    let ready = publication.issued.last().unwrap().directory.clone();
    publication
        .rename_with(&names, "new", |index, step, _| {
            if index == 2 && step == Step::Installed {
                return Err(io::Error::other("rollback now"));
            }
            if index == 2 && step == Step::RollingBack {
                ready.atomic_write(OsStr::new(READY_NAME), b"foreign ready")?;
            }
            Ok(())
        })
        .unwrap_err();
    assert!(publication.rename_recovery_pending());
    assert_eq!(fs::read(location.ready_record()).unwrap(), b"foreign ready");
    assert!(names.load(&publication.metadata.id).unwrap().is_none());
    let registry = location.registries.read(&publication.metadata.id).unwrap();
    assert_eq!(registry[0].host.name.as_deref(), Some("old"));
    publication.cleanup().unwrap();
    assert_eq!(fs::read(location.ready_record()).unwrap(), b"foreign ready");
    assert!(
        location
            .registries
            .read(&publication.metadata.id)
            .unwrap()
            .is_empty()
    );
    assert_no_staging(&ready);
}

#[test]
fn every_incomplete_namespace_scan_refuses_claiming_an_unseen_name() {
    let (_root, location, _names) = fixture("names-scan-limit");
    let publication = publication(&location);
    for limit in [
        ScanLimit::Entries,
        ScanLimit::Rows,
        ScanLimit::MetadataBytes,
    ] {
        let mut scan = location.registries.scan_namespaces();
        scan.limit = Some(limit);
        let error = ensure_scan_available(&scan, &publication.metadata.id, "apparently-unused")
            .unwrap_err();
        assert!(error.to_string().contains("indeterminate"));
    }
}

fn change_for_record(record: &Issued, touched: bool) -> Change {
    Change {
        directory: record.directory.clone(),
        path: record.path.clone(),
        name: record.name.clone(),
        old: Some(Snapshot {
            file: record.file.try_clone().unwrap(),
            bytes: record.bytes.clone(),
        }),
        bytes: b"new record bytes".to_vec(),
        next: None,
        restore: None,
        touched,
        publication_index: Some(0),
    }
}

#[test]
fn partially_written_new_staging_is_owned_and_removed_without_touching_old_record() {
    let (_root, location, _names) = fixture("names-partial-stage");
    let publication = publication(&location);
    let issued = &publication.issued[0];
    let old_identity = locking::file_key(&issued.file).unwrap();
    let mut change = change_for_record(issued, false);
    let (name, file) = change.directory.create_owned_pending().unwrap();
    change.next = Some(Pending { file, name });
    // Model an actual partial write before an I/O failure. Ownership was
    // already recorded; rollback must work without a complete staged value.
    change
        .next
        .as_mut()
        .unwrap()
        .file
        .write_all(b"new")
        .unwrap();
    change.rollback().unwrap();
    assert_eq!(
        locking::file_key(&File::open(&change.path).unwrap()).unwrap(),
        old_identity
    );
    assert_eq!(fs::read(&change.path).unwrap(), issued.bytes);
    assert_no_staging(&change.directory);
    assert_published(&publication, "old");
}

#[test]
fn partially_written_restoration_retries_with_the_same_retained_handle() {
    let (_root, location, _names) = fixture("names-partial-restore");
    let publication = publication(&location);
    let issued = &publication.issued[0];
    let mut change = change_for_record(issued, true);
    let (name, file) = change.directory.create_owned_pending().unwrap();
    change.next = Some(Pending { file, name });
    change
        .next
        .as_mut()
        .unwrap()
        .file
        .write_all(&change.bytes)
        .unwrap();
    change
        .directory
        .remove_owned(OsStr::new(&change.name), &issued.file)
        .unwrap();
    change
        .directory
        .install_owned_pending(
            &change.next.as_ref().unwrap().file,
            OsStr::new(&change.name),
        )
        .unwrap();
    let (name, file) = change.directory.create_owned_pending().unwrap();
    change.restore = Some(Pending { file, name });
    change
        .restore
        .as_mut()
        .unwrap()
        .file
        .write_all(b"bad prefix")
        .unwrap();
    let identity = locking::file_key(&change.restore.as_ref().unwrap().file).unwrap();
    change.rollback().unwrap();
    assert_eq!(
        locking::file_key(&File::open(&change.path).unwrap()).unwrap(),
        identity
    );
    assert_eq!(fs::read(&change.path).unwrap(), issued.bytes);
    assert_no_staging(&change.directory);
    change.cleanup_all().unwrap();
}
