// SPDX-License-Identifier: MPL-2.0

use super::*;
use crate::test_support::TestRuntimeRoot;
use std::fs;

#[test]
fn absent_roots_are_observed_without_creation_and_missing_lock_is_indeterminate() {
    let root = TestRuntimeRoot::new("registry-view-missing").unwrap();
    let absent = root.join("absent");
    let inventory = root.join("inventory");
    let view = RegistryView::open(std::slice::from_ref(&absent), Some(inventory.clone())).unwrap();
    assert!(view.scan_namespaces().candidates.is_empty());
    assert!(view.scan_inventory().candidates.is_empty());
    assert!(!absent.exists());
    assert!(!inventory.exists());
    let private = root.create_private_dir("private").unwrap();
    assert!(RegistryView::open(std::slice::from_ref(&private), None).is_err());
    assert!(!private.join(REGISTRY_LOCK).exists());
}

#[test]
fn readonly_view_does_not_harden_an_existing_nonprivate_directory() {
    let root = TestRuntimeRoot::new("registry-view-acl").unwrap();
    let directory = root.join("ordinary");
    fs::create_dir(&directory).unwrap();
    assert!(Directory::open_existing(&directory, true).is_err());
    assert!(RegistryView::open(std::slice::from_ref(&directory), None).is_err());
    assert!(Directory::open_existing(&directory, true).is_err());
    assert!(fs::read_dir(&directory).unwrap().next().is_none());
}

#[test]
fn inventory_only_view_reuses_candidates_without_inventing_a_namespace() {
    let root = TestRuntimeRoot::new("registry-view-inventory").unwrap();
    let project = root.create_private_dir("project").unwrap();
    let namespace = root.join("namespace");
    let inventory = root.join("inventory");
    let endpoint = EndpointLocation::new(
        &project,
        root.join("ready"),
        RegistrySet::with_inventory(std::slice::from_ref(&namespace), Some(inventory.clone()))
            .unwrap(),
    )
    .unwrap();
    let publication = endpoint
        .prepare(Some("hidden".into()))
        .unwrap()
        .publish()
        .unwrap();
    let view = RegistryView::open(&[], Some(inventory)).unwrap();
    assert!(view.scan_namespaces().candidates.is_empty());
    let scan = view.scan_inventory();
    assert!(scan.issues.is_empty());
    assert!(scan.limit.is_none());
    assert_eq!(scan.candidates.len(), 1);
    assert_eq!(scan.candidates[0].origin(), CandidateOrigin::OwnerInventory);
    assert_eq!(scan.candidates[0].metadata(), publication.metadata());
    assert!(view.observe(&publication.metadata().id).is_err());
    assert!(
        view.observe_namespaces(&publication.metadata().id)
            .unwrap()
            .candidates
            .is_empty()
    );
}

#[test]
fn shared_secondary_and_exact_configured_inventory_use_existing_lock_identities() {
    let root = TestRuntimeRoot::new("registry-view-shared").unwrap();
    let project = root.create_private_dir("project").unwrap();
    let primary = root.join("primary");
    let shared = root.join("shared");
    let inventory = root.join("inventory");
    let endpoint = EndpointLocation::new(
        &project,
        root.join("ready"),
        RegistrySet::with_inventory(&[primary.clone(), shared.clone()], Some(inventory.clone()))
            .unwrap(),
    )
    .unwrap();
    let publication = endpoint.prepare(None).unwrap().publish().unwrap();
    let exact = RegistryView::open(&[primary, shared.clone()], Some(inventory.clone())).unwrap();
    let observed = exact.observe(&publication.metadata().id).unwrap();
    assert!(observed.issues.is_empty());
    assert_eq!(observed.candidates.len(), 3);
    let secondary =
        RegistryView::open(&[root.join("other-primary"), shared], Some(inventory)).unwrap();
    assert_eq!(secondary.scan_namespaces().candidates.len(), 1);
    assert!(secondary.observe(&publication.metadata().id).is_err());
    assert_eq!(
        secondary
            .observe_namespaces(&publication.metadata().id)
            .unwrap()
            .candidates
            .len(),
        1
    );
    assert!(!root.join("other-primary").exists());
}

#[test]
fn namespace_aliases_deduplicate_but_inventory_alias_is_refused() {
    let root = TestRuntimeRoot::new("registry-view-alias").unwrap();
    let namespace = root.join("namespace");
    RegistrySet::open_fixture(std::slice::from_ref(&namespace)).unwrap();
    let alias = namespace.join(".");
    let view = RegistryView::open(&[namespace.clone(), alias.clone()], None).unwrap();
    assert_eq!(view.registries.0.len(), 1);
    assert!(RegistryView::open(&[namespace], Some(alias)).is_err());
}

#[test]
fn bounded_scanner_reports_bad_rows_without_rewriting_them() {
    let root = TestRuntimeRoot::new("registry-view-corrupt").unwrap();
    let path = root.join("namespace");
    RegistrySet::open_fixture(std::slice::from_ref(&path)).unwrap();
    let name = format!("{}.json", "a".repeat(crate::workspace::WORKSPACE_ID_LENGTH));
    Directory::open_existing(&path, true)
        .unwrap()
        .atomic_write(OsStr::new(&name), b"broken")
        .unwrap();
    let view = RegistryView::open(std::slice::from_ref(&path), None).unwrap();
    let scan = view.scan_namespaces();
    assert!(scan.candidates.is_empty());
    assert_eq!(scan.issues.len(), 1);
    assert_eq!(fs::read(path.join(name)).unwrap(), b"broken");
}

#[test]
fn unscoped_ready_can_be_observed_but_not_retired_without_namespace_identity() {
    let root = TestRuntimeRoot::new("registry-view-ready").unwrap();
    let project = root.create_private_dir("project").unwrap();
    let ready = root.join("ready");
    let mut metadata = EndpointMetadata::new(&project.canonicalize().unwrap(), None).unwrap();
    metadata.process.creation_time -= 1;
    Directory::open(&ready, true)
        .unwrap()
        .atomic_write(OsStr::new(READY_NAME), &metadata.to_json().unwrap())
        .unwrap();
    let view = RegistryView::open(&[], None).unwrap();
    let candidate = view
        .observe_ready(&project, ready.clone())
        .unwrap()
        .unwrap();
    assert_eq!(candidate.metadata(), &metadata);
    assert_eq!(candidate.origin(), CandidateOrigin::ConfiguredReady);
    let Inspection::Stale(evidence) = candidate.inspect_process().unwrap() else {
        panic!("wrong creation time must be stale");
    };
    assert!(evidence.remove_observed().is_err());
    assert!(ready.join(READY_NAME).exists());
}
