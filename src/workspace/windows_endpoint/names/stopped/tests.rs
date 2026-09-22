// SPDX-License-Identifier: MPL-2.0

use super::*;
use crate::{
    test_support::TestRuntimeRoot,
    workspace::{
        recent_history::{RecentEntry, encode_recents},
        windows_catalog::snapshot_with_history,
        windows_location::{CapturedRoots, LocationInputs, ResolvedLayout},
    },
};
use std::{cell::RefCell, fs};

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}
fn fixture(label: &str) -> (TestRuntimeRoot, ResolvedLayout, StoppedNameEdit) {
    let root = TestRuntimeRoot::new(label).unwrap();
    let project = root.create_private_dir("project").unwrap();
    let layout = ResolvedLayout::resolve(LocationInputs {
        state_root: project.join(".runyte"),
        project_root: project,
        reserved_user_roots: vec![root.join("config")],
        roots: CapturedRoots {
            cache_home: Some(root.join("cache")),
            inventory_override: Some(root.join("inventory")),
            ..CapturedRoots::default()
        },
    })
    .unwrap();
    let history_path = layout
        .cache_root()
        .unwrap()
        .unwrap()
        .join("workspaces.json");
    Directory::open(history_path.parent().unwrap(), true)
        .unwrap()
        .atomic_write(
            OsStr::new("workspaces.json"),
            &encode_recents(&[RecentEntry::new(
                layout.project_root().into(),
                Some("old".into()),
                Some(2),
                Some(10),
            )])
            .unwrap(),
        )
        .unwrap();
    let edit = StoppedNameEdit::new(StoppedNameSelection {
        location: layout.publication_location().unwrap(),
        store: NameStore::open(layout.state_root()).unwrap(),
        scope: layout.discovery_scope().clone(),
        configured_state: ".runyte".into(),
        history_path,
        expected_name: None,
        cached_name: Some("old".into()),
    });
    (root, layout, edit)
}
fn id(edit: &StoppedNameEdit) -> String {
    crate::workspace::workspace_id(&edit.selection.location.project)
}
fn seed(edit: &mut StoppedNameEdit, name: &str) {
    edit.selection
        .store
        .store_if_absent(&edit.selection.location.registries, &id(edit), name)
        .unwrap();
    edit.selection.expected_name = Some(name.into());
}
fn stored(edit: &StoppedNameEdit) -> Option<String> {
    edit.selection.store.load(&id(edit)).unwrap()
}
fn no_staging(edit: &StoppedNameEdit) {
    let (entries, truncated) = edit.selection.store.directory.entries(32).unwrap();
    assert!(!truncated);
    assert!(
        entries
            .iter()
            .all(|name| !name.to_string_lossy().starts_with(".runyte-transaction-"))
    );
}

#[test]
fn read_only_store_requires_existing_stable_authority_and_never_creates_it() {
    let root = TestRuntimeRoot::new("name-read-existing").unwrap();
    let state = root.join("state");
    let id = crate::workspace::workspace_id(&root.path().canonicalize().unwrap());
    assert_eq!(NameStore::read_existing(&state, &id).unwrap(), None);
    assert!(!state.exists());
    let names = NameStore::open(&state).unwrap();
    assert_eq!(NameStore::read_existing(&state, &id).unwrap(), None);
    assert!(!names.path.join(STORE_LOCK).exists());
    let leaf = format!("{id}.json");
    names
        .directory
        .atomic_write(OsStr::new(&leaf), b"\"name\"")
        .unwrap();
    assert!(
        NameStore::read_existing(&state, &id)
            .unwrap_err()
            .to_string()
            .contains("stable lock")
    );
    assert!(!names.path.join(STORE_LOCK).exists());
    assert_eq!(names.load(&id).unwrap().as_deref(), Some("name"));
    assert_eq!(
        NameStore::read_existing(&state, &id).unwrap().as_deref(),
        Some("name")
    );
    {
        let _locked = names.lock().unwrap();
        fs::remove_file(names.path.join(&leaf)).unwrap();
        assert_eq!(
            NameStore::read_existing(&state, &id).unwrap_err().kind(),
            io::ErrorKind::WouldBlock
        );
    }
    assert_eq!(NameStore::read_existing(&state, &id).unwrap(), None);
    names
        .directory
        .atomic_write(OsStr::new(&leaf), b"invalid")
        .unwrap();
    assert!(
        NameStore::read_existing(&state, &id)
            .unwrap_err()
            .to_string()
            .contains("malformed stored")
    );
    assert_eq!(fs::read(names.path.join(&leaf)).unwrap(), b"invalid");
}

#[test]
fn read_only_store_refuses_replaced_lock_without_changing_its_record() {
    let (_root, layout, mut edit) = fixture("name-read-lock-change");
    seed(&mut edit, "old");
    let result = NameStore::read_existing_with(layout.state_root(), &id(&edit), |names| {
        names
            .directory
            .atomic_write(OsStr::new(STORE_LOCK), b"replacement")
    });
    assert!(result.unwrap_err().to_string().contains("changed identity"));
    assert_eq!(stored(&edit).as_deref(), Some("old"));
}

#[test]
fn verified_name_commit_survives_cache_failure_and_is_used_by_listing_and_startup() {
    runtime().block_on(async {
        let (_root, layout, mut edit) = fixture("name-authority");
        seed(&mut edit, "old");
        let before = fs::read(&edit.selection.history_path).unwrap();
        let committed = edit
            .rename_with(
                "new",
                |_, _| Ok(()),
                || Err(io::Error::other("injected cache failure")),
            )
            .unwrap();
        assert_eq!(committed.name, "new");
        assert!(
            committed
                .cache_error
                .unwrap()
                .contains("injected cache failure")
        );
        assert_eq!(fs::read(&edit.selection.history_path).unwrap(), before);
        assert_eq!(stored(&edit).as_deref(), Some("new"));
        let listing = snapshot_with_history(&layout, Path::new(".runyte"), false)
            .await
            .unwrap();
        assert_eq!(listing.entries()[0].row().name.as_deref(), Some("new"));
        assert!(listing.select(Path::new("old"), None).unwrap().is_none());
        assert!(listing.select(Path::new("new"), None).unwrap().is_some());
        // Original cache bytes remain the compare baseline, not the overlay.
        assert_eq!(listing.remembered()[0].name.as_deref(), Some("old"));
        let prepared = edit
            .selection
            .location
            .prepare_named(&edit.selection.store, None)
            .unwrap();
        assert_eq!(prepared.metadata().name.as_deref(), Some("new"));
        drop(prepared);
        no_staging(&edit);
    });
}

#[test]
fn startup_is_serialized_and_all_exact_occupants_refuse_stopped_rename() {
    let (_root, layout, mut edit) = fixture("name-vacancy");
    seed(&mut edit, "old");
    let location = edit.selection.location.clone();
    let names = edit.selection.store.clone();
    let mut checked = false;
    edit.rename_with(
        "new",
        |step, _| {
            if step == Step::Removed {
                checked = true;
                assert_eq!(
                    location.prepare_named(&names, None).unwrap_err().kind(),
                    io::ErrorKind::WouldBlock
                );
            }
            Ok(())
        },
        || Ok(()),
    )
    .unwrap();
    assert!(checked);
    let publication = location
        .prepare_named(&names, None)
        .unwrap()
        .publish()
        .unwrap();
    assert!(edit.rename("occupied").is_err());
    assert_eq!(stored(&edit).as_deref(), Some("new"));
    let ready_bytes = fs::read(location.ready_record()).unwrap();
    // Registry and exact own-inventory occupancy remain evidence without ready.
    fs::remove_file(location.ready_record()).unwrap();
    assert!(edit.rename("registry-only").is_err());
    for namespace in layout.namespace_roots() {
        fs::remove_file(namespace.join(format!("{}.json", publication.metadata().id))).unwrap();
    }
    assert!(edit.rename("inventory-only").is_err());
    drop(publication);
    Directory::open(&location.directory, true)
        .unwrap()
        .atomic_write(OsStr::new(READY_NAME), &ready_bytes)
        .unwrap();
    assert!(edit.rename("ready-only").is_err());
    Directory::open(&location.directory, true)
        .unwrap()
        .atomic_write(OsStr::new(READY_NAME), b"malformed")
        .unwrap();
    assert!(edit.rename("malformed-ready").is_err());
    fs::remove_file(location.ready_record()).unwrap();
    edit.rename("vacant-again").unwrap();
}

#[test]
fn stale_stored_or_fallback_names_and_forgotten_membership_are_refused() {
    let (_root, _layout, mut edit) = fixture("name-stale-intent");
    update_recents_if_changed(&edit.selection.history_path, |entries| {
        entries[0].name = Some("new-fallback".into());
        Ok(())
    })
    .unwrap();
    assert!(
        edit.rename("requested")
            .unwrap_err()
            .to_string()
            .contains("fallback session name changed")
    );
    assert_eq!(stored(&edit), None);
    seed(&mut edit, "explicit");
    update_recents_if_changed(&edit.selection.history_path, |entries| {
        entries[0].number = Some(8);
        entries[0].last_active_unix_seconds = Some(99);
        Ok(())
    })
    .unwrap();
    edit.rename("requested").unwrap();
    let current = read_recents(Some(&edit.selection.history_path)).unwrap();
    assert_eq!(current[0].name.as_deref(), Some("new-fallback"));
    assert_eq!(current[0].number, Some(8));
    assert_eq!(current[0].last_active_unix_seconds, Some(99));
    edit.selection.expected_name = Some("obsolete".into());
    assert!(
        edit.rename("stale")
            .unwrap_err()
            .to_string()
            .contains("stored session name changed")
    );
    edit.selection.expected_name = Some("requested".into());
    update_recents_if_changed(&edit.selection.history_path, |entries| {
        entries.clear();
        Ok(())
    })
    .unwrap();
    assert!(
        edit.rename("forgotten")
            .unwrap_err()
            .to_string()
            .contains("forgotten")
    );
    assert!(
        read_recents(Some(&edit.selection.history_path))
            .unwrap()
            .is_empty()
    );
    assert_eq!(stored(&edit).as_deref(), Some("requested"));
}

#[test]
fn failure_boundaries_restore_old_name_and_preserve_original_error_type() {
    for failed in [
        Step::Staged,
        Step::Removing,
        Step::Removed,
        Step::Installed,
        Step::Synced,
    ] {
        let (_root, _layout, mut edit) = fixture("name-rollback");
        seed(&mut edit, "old");
        let result = edit.rename_with(
            "new",
            |step, _| {
                if step == failed {
                    Err(io::Error::new(
                        io::ErrorKind::PermissionDenied,
                        "injected boundary",
                    ))
                } else {
                    Ok(())
                }
            },
            || Ok(()),
        );
        let error = result.unwrap_err();
        assert_eq!(
            error.downcast_ref::<io::Error>().unwrap().kind(),
            io::ErrorKind::PermissionDenied
        );
        assert!(!edit.recovery_pending());
        assert_eq!(stored(&edit).as_deref(), Some("old"));
        no_staging(&edit);
    }
    let (_root, _layout, mut edit) = fixture("name-retained-rollback");
    seed(&mut edit, "old");
    let error = edit
        .rename_with(
            "new",
            |step, _| {
                if step == Step::Installed || step == Step::RollingBack {
                    Err(io::Error::new(
                        io::ErrorKind::PermissionDenied,
                        "injected retained recovery",
                    ))
                } else {
                    Ok(())
                }
            },
            || Ok(()),
        )
        .unwrap_err();
    assert_eq!(
        error.downcast_ref::<io::Error>().unwrap().kind(),
        io::ErrorKind::PermissionDenied
    );
    assert!(edit.recovery_pending());
    assert!(edit.rename("another").is_err());
    let publication = edit
        .selection
        .location
        .prepare_named(&edit.selection.store, None)
        .unwrap()
        .publish()
        .unwrap();
    assert!(edit.retry_recovery().is_err());
    assert_eq!(stored(&edit).as_deref(), Some("new"));
    drop(publication);
    edit.retry_recovery().unwrap();
    assert_eq!(stored(&edit).as_deref(), Some("old"));
    no_staging(&edit);
}

#[test]
fn foreign_replacement_survives_until_explicit_removal_allows_owned_recovery() {
    let (_root, _layout, mut edit) = fixture("name-foreign-replace");
    seed(&mut edit, "old");
    let store = edit.selection.store.clone();
    let leaf = format!("{}.json", id(&edit));
    let foreign = RefCell::new(None);
    let error = edit
        .rename_with(
            "new",
            |step, _| {
                if step == Step::Removed {
                    *foreign.borrow_mut() = Some(
                        store
                            .directory
                            .atomic_write_owned(OsStr::new(&leaf), b"\"foreign\"")?,
                    );
                }
                Ok(())
            },
            || Ok(()),
        )
        .unwrap_err();
    assert!(error.to_string().contains("unknown"));
    assert!(edit.recovery_pending());
    assert_eq!(stored(&edit).as_deref(), Some("foreign"));
    assert!(edit.retry_recovery().is_err());
    store
        .directory
        .remove_owned(OsStr::new(&leaf), foreign.borrow().as_ref().unwrap())
        .unwrap();
    edit.retry_recovery().unwrap();
    assert_eq!(stored(&edit).as_deref(), Some("old"));
    no_staging(&edit);
}

#[test]
fn partial_staging_and_restore_reuse_the_single_retained_change() {
    let (_root, _layout, mut edit) = fixture("name-partial-files");
    seed(&mut edit, "old");
    let (old, bytes, _) = edit
        .selection
        .store
        .read_unlocked(&id(&edit))
        .unwrap()
        .unwrap();
    let old_key = locking::file_key(&old).unwrap();
    let store = edit.selection.store.clone();
    let leaf = format!("{}.json", id(&edit));
    let (name, mut file) = store.directory.create_owned_pending().unwrap();
    file.write_all(b"\"n").unwrap();
    edit.recovery = Some(Change {
        directory: store.directory.clone(),
        path: store.path.join(&leaf),
        name: leaf.clone(),
        old: Some(Snapshot { file: old, bytes }),
        bytes: b"\"new\"".to_vec(),
        next: Some(Pending { name, file }),
        restore: None,
        touched: false,
        publication_index: None,
    });
    edit.retry_recovery().unwrap();
    assert_eq!(
        locking::file_key(&store.read_unlocked(&id(&edit)).unwrap().unwrap().0).unwrap(),
        old_key
    );
    no_staging(&edit);

    edit.rename_with(
        "new",
        |step, _| {
            if step == Step::Installed || step == Step::RollingBack {
                Err(io::Error::other("retain"))
            } else {
                Ok(())
            }
        },
        || Ok(()),
    )
    .unwrap_err();
    let change = edit.recovery.as_mut().unwrap();
    let (name, mut file) = store.directory.create_owned_pending().unwrap();
    file.write_all(b"\"o").unwrap();
    let restore_key = locking::file_key(&file).unwrap();
    change.restore = Some(Pending { name, file });
    edit.retry_recovery().unwrap();
    let (restored, _, value) = store.read_unlocked(&id(&edit)).unwrap().unwrap();
    assert_eq!(value, "old");
    assert_eq!(locking::file_key(&restored).unwrap(), restore_key);
    no_staging(&edit);
}

#[test]
fn final_owner_drop_retries_retained_recovery_without_reverting_a_verified_commit() {
    let (_root, _layout, mut edit) = fixture("name-owner-drop");
    seed(&mut edit, "old");
    let store = edit.selection.store.clone();
    let id = id(&edit);
    edit.rename_with(
        "new",
        |step, _| {
            if step == Step::Installed || step == Step::RollingBack {
                Err(io::Error::other("retain"))
            } else {
                Ok(())
            }
        },
        || Ok(()),
    )
    .unwrap_err();
    assert!(edit.recovery_pending());
    drop(edit);
    assert_eq!(store.load(&id).unwrap().as_deref(), Some("old"));
    let (_other_root, _other_layout, mut committed) = fixture("name-committed-drop");
    let names = committed.selection.store.clone();
    let committed_id = crate::workspace::workspace_id(&committed.selection.location.project);
    committed.rename("verified").unwrap();
    assert!(!committed.recovery_pending());
    drop(committed);
    assert_eq!(
        names.load(&committed_id).unwrap().as_deref(),
        Some("verified")
    );
}

#[test]
fn unwinding_observer_retains_recovery_for_the_worker_owner() {
    let (_root, _layout, mut edit) = fixture("name-unwind-owner");
    seed(&mut edit, "old");
    let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = edit.rename_with(
            "new",
            |step, _| {
                assert!(step != Step::Installed, "observer unwound after install");
                Ok(())
            },
            || Ok(()),
        );
    }));
    assert!(panicked.is_err());
    assert!(edit.recovery_pending());
    assert!(edit.rename("another").is_err());
    edit.retry_recovery().unwrap();
    assert_eq!(stored(&edit).as_deref(), Some("old"));
    no_staging(&edit);
}

#[test]
fn configured_competitor_authority_and_incomplete_namespace_refuse_name_claims() {
    let (root, layout, mut edit) = fixture("name-competitors");
    let project = root
        .create_private_dir("competitor")
        .unwrap()
        .canonicalize()
        .unwrap();
    let competing = ResolvedLayout::from_scope(
        layout.discovery_scope().clone(),
        &project,
        project.join(".runyte"),
    )
    .unwrap();
    let names = NameStore::open(competing.state_root()).unwrap();
    names
        .store_if_absent(
            &competing.publication_location().unwrap().registries,
            &crate::workspace::workspace_id(&project),
            "reserved",
        )
        .unwrap();
    assert_eq!(
        NameStore::read_existing(
            competing.state_root(),
            &crate::workspace::workspace_id(&project)
        )
        .unwrap()
        .as_deref(),
        Some("reserved")
    );
    update_recents_if_changed(&edit.selection.history_path, |entries| {
        entries.push(RecentEntry::new(
            project.clone(),
            Some("stale-cache".into()),
            None,
            None,
        ));
        Ok(())
    })
    .unwrap();
    assert!(
        edit.rename("reserved")
            .unwrap_err()
            .to_string()
            .contains("reserved")
    );
    assert_eq!(stored(&edit), None);
    let unrelated = crate::workspace::workspace_id(&root.join("unrelated"));
    Directory::open(&layout.namespace_roots()[0], true)
        .unwrap()
        .atomic_write(OsStr::new(&format!("{unrelated}.json")), b"malformed")
        .unwrap();
    assert!(edit.rename("otherwise-free").is_err());
    assert_eq!(stored(&edit), None);
}
