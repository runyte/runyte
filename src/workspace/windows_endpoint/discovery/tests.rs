// SPDX-License-Identifier: MPL-2.0

use super::*;
use crate::test_support::TestRuntimeRoot;
use std::{fs, io::Write};

fn location(root: &Path) -> EndpointLocation {
    let project = root.join("project");
    fs::create_dir_all(&project).unwrap();
    EndpointLocation::new(
        &project,
        root.join("endpoint"),
        RegistrySet::open(&[root.join("registry")]).unwrap(),
    )
    .unwrap()
}

fn stale_publication(location: &EndpointLocation) -> Publication {
    let mut prepared = location.prepare(None).unwrap();
    // Same PID, conclusively different process creation identity. No process
    // is terminated and no fixture needs an unowned historical PID.
    prepared.metadata.process.creation_time ^= 1;
    prepared.publish().unwrap()
}

fn stale(candidate: &Candidate) -> StaleEvidence<'_> {
    match candidate.inspect_process().unwrap() {
        Inspection::Stale(evidence) => evidence,
        Inspection::Present(_) => panic!("fixture must have a reused process identity"),
    }
}

#[test]
fn configured_ready_and_exact_rows_retire_independently_before_reprepare() {
    let root = TestRuntimeRoot::new("discovery-retire").unwrap();
    let location = location(root.path());
    assert!(location.observe_ready().unwrap().is_none());
    let publication = stale_publication(&location);
    let observed = location
        .registries
        .observe(&publication.metadata.id)
        .unwrap();
    assert!(observed.issues.is_empty());
    assert_eq!(observed.candidates.len(), 1);
    assert_eq!(
        observed.candidates[0].origin(),
        CandidateOrigin::ConfiguredNamespace
    );
    let ready = location.observe_ready().unwrap().unwrap();
    assert_eq!(ready.origin(), CandidateOrigin::ConfiguredReady);
    assert_eq!(stale(&ready).reason(), StaleReason::Reused);
    assert_eq!(stale(&ready).remove_observed().unwrap(), Removal::Removed);
    assert!(!location.ready_record().exists());
    assert!(
        location.prepare(None).is_err(),
        "registry row still occupies namespace"
    );
    assert_eq!(
        stale(&observed.candidates[0]).remove_observed().unwrap(),
        Removal::Removed
    );
    assert_eq!(
        stale(&observed.candidates[0]).remove_observed().unwrap(),
        Removal::Missing
    );
    drop(location.prepare(None).unwrap());
}

#[test]
fn cleanup_requires_locks_repeated_process_proof_exact_file_and_exact_bytes() {
    let root = TestRuntimeRoot::new("discovery-rechecks").unwrap();
    let location = location(root.path());
    let publication = stale_publication(&location);
    let mut observed = location
        .registries
        .observe(&publication.metadata.id)
        .unwrap();
    let candidate = observed.candidates.pop().unwrap();
    let locks = location
        .registries
        .identity_locks(&publication.metadata.id)
        .unwrap();
    assert_eq!(
        stale(&candidate).remove_observed().unwrap_err().kind(),
        io::ErrorKind::WouldBlock
    );
    drop(locks);
    assert_eq!(
        stale(&candidate)
            .remove_with(|_| Ok(PinResult::Pinned(
                PinnedProcess::open_peer(std::process::id()).unwrap()
            )))
            .unwrap(),
        Removal::ProcessPresent
    );
    assert_eq!(
        stale(&candidate)
            .remove_with(|_| Err(io::Error::from(io::ErrorKind::PermissionDenied)))
            .unwrap_err()
            .kind(),
        io::ErrorKind::PermissionDenied
    );
    assert!(candidate.issued.path.exists());

    // Same file identity and incarnation, changed bytes: this is not the
    // exact observation and must not be removed.
    let mut changed = candidate.registry_record().unwrap().clone();
    changed.host.name = Some("changed".into());
    fs::OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(&candidate.issued.path)
        .unwrap()
        .write_all(&changed.to_json().unwrap())
        .unwrap();
    assert_eq!(
        stale(&candidate).remove_observed().unwrap(),
        Removal::Changed
    );
    assert_eq!(
        fs::read(&candidate.issued.path).unwrap(),
        changed.to_json().unwrap()
    );

    // A different file containing identical original bytes is also protected.
    candidate
        .issued
        .directory
        .atomic_write(OsStr::new(&candidate.issued.name), &candidate.issued.bytes)
        .unwrap();
    assert_eq!(
        stale(&candidate).remove_observed().unwrap(),
        Removal::Changed
    );
    assert_eq!(
        fs::read(&candidate.issued.path).unwrap(),
        candidate.issued.bytes
    );
}

#[test]
fn gone_evidence_is_typed_and_denied_inspection_never_authorizes_removal() {
    let root = TestRuntimeRoot::new("discovery-inspection").unwrap();
    let location = location(root.path());
    let publication = location.prepare(None).unwrap().publish().unwrap();
    let observed = location
        .registries
        .observe(&publication.metadata.id)
        .unwrap();
    let candidate = &observed.candidates[0];
    assert!(matches!(
        candidate.inspect_process().unwrap(),
        Inspection::Present(_)
    ));
    assert_eq!(
        candidate
            .inspect_with(|_| Err(io::Error::from(io::ErrorKind::PermissionDenied)))
            .unwrap_err()
            .kind(),
        io::ErrorKind::PermissionDenied
    );
    let Inspection::Stale(evidence) = candidate.inspect_with(|_| Ok(PinResult::Gone)).unwrap()
    else {
        panic!("gone is conclusive")
    };
    assert_eq!(evidence.reason(), StaleReason::Gone);
    assert_eq!(evidence.remove_observed().unwrap(), Removal::ProcessPresent);
    assert!(candidate.issued.path.exists());
}

#[test]
fn hidden_inventory_row_cleanup_never_reconstructs_or_removes_its_namespace() {
    let root = TestRuntimeRoot::new("discovery-inventory").unwrap();
    let project = root.join("project");
    fs::create_dir(&project).unwrap();
    let inventory = root.join("inventory");
    let registries_a =
        RegistrySet::with_inventory(&[root.join("a")], Some(inventory.clone())).unwrap();
    let registries_b = RegistrySet::with_inventory(&[root.join("b")], Some(inventory)).unwrap();
    let a = EndpointLocation::new(&project, root.join("endpoint-a"), registries_a.clone()).unwrap();
    let b = EndpointLocation::new(&project, root.join("endpoint-b"), registries_b.clone()).unwrap();
    let publication_a = stale_publication(&a);
    let publication_b = stale_publication(&b);
    assert_eq!(registries_a.scan_namespaces().candidates.len(), 1);
    let inventory = registries_a.scan_inventory();
    assert_eq!(inventory.candidates.len(), 2);
    let hidden = inventory
        .candidates
        .iter()
        .find(|candidate| candidate.metadata.incarnation == publication_b.metadata.incarnation)
        .unwrap();
    assert_eq!(hidden.origin(), CandidateOrigin::OwnerInventory);
    let b_namespace = registries_b.0.iter().find(|root| !root.inventory).unwrap();
    // Cleanup of this hidden inventory observation must not acquire B's
    // namespace lock, much less follow its reported ready path for deletion.
    let _namespace_lock = locking::acquire(std::slice::from_ref(b_namespace), |_| {
        REGISTRY_LOCK.to_owned()
    })
    .unwrap();
    assert_eq!(stale(hidden).remove_observed().unwrap(), Removal::Removed);
    assert!(b.ready_record().exists());
    assert!(
        b_namespace
            .path
            .join(format!("{}.json", publication_b.metadata.id))
            .exists()
    );
    let left = registries_a.scan_inventory();
    assert_eq!(left.candidates.len(), 1);
    assert_eq!(
        left.candidates[0].metadata.incarnation,
        publication_a.metadata.incarnation
    );
}

#[test]
fn targeted_observation_ignores_unrelated_bad_rows_and_inventory_scan_limits() {
    let root = TestRuntimeRoot::new("discovery-targeted").unwrap();
    let project = root.join("project");
    fs::create_dir(&project).unwrap();
    let registries =
        RegistrySet::with_inventory(&[root.join("registry")], Some(root.join("inventory")))
            .unwrap();
    let location =
        EndpointLocation::new(&project, root.join("endpoint"), registries.clone()).unwrap();
    let publication = stale_publication(&location);
    let inventory = registries.0.iter().find(|root| root.inventory).unwrap();
    for index in 0..8 {
        inventory
            .directory
            .create_new(OsStr::new(&format!("{index:032x}-{}.json", "0".repeat(32))))
            .unwrap()
            .write_all(b"malformed")
            .unwrap();
    }
    assert_eq!(
        registries
            .scan(true, 0, MAX_SCAN_ROWS, MAX_SCAN_BYTES)
            .limit,
        Some(ScanLimit::Entries)
    );
    assert_eq!(
        registries
            .scan(true, MAX_SCAN_ENTRIES, 0, MAX_SCAN_BYTES)
            .limit,
        Some(ScanLimit::Rows)
    );
    let observed = registries.observe(&publication.metadata.id).unwrap();
    assert!(observed.issues.is_empty());
    assert_eq!(observed.candidates.len(), 2);
    assert!(
        observed
            .candidates
            .iter()
            .any(|candidate| candidate.origin() == CandidateOrigin::OwnerInventory)
    );
    assert!(registries.observe("../escape").is_err());
    assert!(
        registries
            .observe(&"f".repeat(32))
            .unwrap()
            .candidates
            .is_empty()
    );
    // An error at our own key is retained, not treated as a missing host.
    let own = observed
        .candidates
        .iter()
        .find(|candidate| candidate.origin() == CandidateOrigin::OwnerInventory)
        .unwrap();
    own.issued
        .directory
        .atomic_write(OsStr::new(&own.issued.name), b"bad")
        .unwrap();
    let observed = registries.observe(&publication.metadata.id).unwrap();
    assert_eq!(observed.candidates.len(), 1);
    assert_eq!(observed.issues.len(), 1);
}

#[test]
fn scan_reports_malformed_oversized_wrong_identity_and_nonprivate_rows() {
    let root = TestRuntimeRoot::new("discovery-malformed").unwrap();
    let location = location(root.path());
    let registry = &location.registries.0[0];
    let valid = EndpointMetadata::new(&location.project, None).unwrap();
    let wrong = RegistryRecord {
        host: valid,
        ready_record_bytes: encode_path(&location.ready_record()),
    }
    .to_json()
    .unwrap();
    for (name, bytes) in [
        (format!("{}.json", "0".repeat(32)), b"bad".to_vec()),
        (
            format!("{}.json", "1".repeat(32)),
            vec![b' '; MAX_METADATA_BYTES + 1],
        ),
        (format!("{}.json", "2".repeat(32)), wrong),
    ] {
        registry
            .directory
            .create_new(OsStr::new(&name))
            .unwrap()
            .write_all(&bytes)
            .unwrap();
    }
    // Hard links are refused even if the source file has the expected ACL.
    let linked = format!("{}.json", "3".repeat(32));
    registry
        .directory
        .create_new(OsStr::new(&linked))
        .unwrap()
        .write_all(b"bad")
        .unwrap();
    fs::hard_link(
        registry.path.join(&linked),
        registry.path.join("unrelated-link"),
    )
    .unwrap();
    let scan = location.registries.scan_namespaces();
    assert!(scan.candidates.is_empty());
    assert_eq!(scan.issues.len(), 4);
    assert!(scan.limit.is_none());
    assert!(scan.issues.iter().all(|issue| {
        issue
            .name
            .as_ref()
            .is_some_and(|name| registry.path.join(name).exists())
    }));
}

#[test]
fn cumulative_read_budget_charges_oversize_errors_and_verification_rereads() {
    let root = TestRuntimeRoot::new("discovery-read-budget").unwrap();
    let location = location(root.path());
    let publication = stale_publication(&location);
    let candidate = location
        .registries
        .observe(&publication.metadata.id)
        .unwrap()
        .candidates
        .pop()
        .unwrap();
    let size = candidate.issued.bytes.len();
    // One read fits; pathname verification cannot fit in this budget.
    let scan = location
        .registries
        .scan(false, MAX_SCAN_ENTRIES, MAX_SCAN_ROWS, size + 1);
    assert!(scan.candidates.is_empty());
    assert_eq!(scan.limit, Some(ScanLimit::MetadataBytes));
    let scan = location
        .registries
        .scan(false, MAX_SCAN_ENTRIES, MAX_SCAN_ROWS, size * 2 + 1);
    assert_eq!(scan.candidates.len(), 1);
    assert!(scan.limit.is_none());

    let directory = &candidate.issued.directory;
    directory
        .create_new(OsStr::new("oversized"))
        .unwrap()
        .write_all(&vec![b' '; MAX_METADATA_BYTES + 1])
        .unwrap();
    let mut budget = ReadBudget {
        remaining: MAX_METADATA_BYTES + 5,
    };
    assert!(budget.read(directory, "oversized").is_err());
    assert_eq!(
        budget.remaining, 4,
        "oversized rows still consume their full attempted bytes"
    );
    let error = budget.read(directory, &candidate.issued.name).unwrap_err();
    assert!(error.get_ref().unwrap().is::<ReadBudgetExhausted>());
    assert_eq!(budget.remaining, 0);
}

#[tokio::test(flavor = "current_thread")]
async fn native_peer_proof_is_distinct_from_process_presence_and_protocol_compatibility() {
    let root = TestRuntimeRoot::new("discovery-native-proof").unwrap();
    let location = location(root.path());
    let mut prepared = location.prepare(None).unwrap();
    prepared.metadata.protocol = crate::protocol::VERSION + 1;
    let listener = windows_pipe::Listener::bind(prepared).unwrap();
    let observed = location
        .registries
        .observe(&listener.metadata().id)
        .unwrap();
    let candidate = &observed.candidates[0];
    assert!(matches!(
        candidate.inspect_process().unwrap(),
        Inspection::Present(_)
    ));
    let host = candidate
        .authenticate(Instant::now() + Duration::from_secs(2))
        .await
        .unwrap();
    assert_eq!(host.metadata(), listener.metadata());
    assert_eq!(host.peer().identity(), listener.metadata().process);
    assert!(!host.speaks_current_protocol());
}

#[tokio::test(flavor = "current_thread")]
async fn missing_busy_and_wrong_identity_pipes_are_indeterminate_and_never_removed() {
    let root = TestRuntimeRoot::new("discovery-indeterminate").unwrap();
    let missing = location(&root.join("missing"));
    let publication = missing.prepare(None).unwrap().publish().unwrap();
    let observed = missing
        .registries
        .observe(&publication.metadata.id)
        .unwrap();
    let candidate = &observed.candidates[0];
    assert!(
        candidate
            .authenticate(Instant::now() + Duration::from_millis(20))
            .await
            .is_err()
    );
    assert!(candidate.issued.path.exists());
    assert!(matches!(
        candidate.inspect_process().unwrap(),
        Inspection::Present(_)
    ));

    let busy = location(&root.join("busy"));
    let listener = windows_pipe::Listener::bind(busy.prepare(None).unwrap()).unwrap();
    let _first =
        windows_pipe::connect(listener.metadata(), Instant::now() + Duration::from_secs(2))
            .await
            .unwrap();
    let observed = busy.registries.observe(&listener.metadata().id).unwrap();
    assert!(
        observed.candidates[0]
            .authenticate(Instant::now() + Duration::from_millis(20))
            .await
            .is_err()
    );
    assert!(busy.ready_record().exists());

    let wrong = location(&root.join("wrong"));
    let mut prepared = wrong.prepare(None).unwrap();
    prepared.metadata.process.creation_time ^= 1;
    let listener = windows_pipe::Listener::bind(prepared).unwrap();
    let observed = wrong.registries.observe(&listener.metadata().id).unwrap();
    assert_eq!(
        observed.candidates[0]
            .authenticate(Instant::now() + Duration::from_secs(2))
            .await
            .unwrap_err()
            .kind(),
        io::ErrorKind::PermissionDenied
    );
    assert!(wrong.ready_record().exists());
}

#[tokio::test(flavor = "current_thread")]
async fn exhausted_probe_budget_preserves_every_candidate_scan_error_and_limit() {
    let root = TestRuntimeRoot::new("discovery-probe-budget").unwrap();
    let location = location(root.path());
    let publication = location.prepare(None).unwrap().publish().unwrap();
    let mut scan = location
        .registries
        .observe(&publication.metadata.id)
        .unwrap();
    scan.issues.push(ScanIssue {
        root: root.path().to_owned(),
        name: None,
        error: io::Error::from(io::ErrorKind::PermissionDenied),
    });
    scan.limit = Some(ScanLimit::Entries);
    let probed = scan.probe_until(Instant::now()).await;
    assert_eq!(probed.candidates.len(), 1);
    assert!(matches!(
        probed.candidates[0].authentication,
        Err(ProbeFailure::BudgetExhausted)
    ));
    assert_eq!(probed.issues.len(), 1);
    assert_eq!(probed.limit, Some(ScanLimit::Entries));
    assert!(probed.candidates[0].candidate.issued.path.exists());
    let probed = location
        .registries
        .observe(&publication.metadata.id)
        .unwrap()
        .probe_until(Instant::now() + Duration::from_millis(20))
        .await;
    assert!(matches!(
        probed.candidates[0].authentication,
        Err(ProbeFailure::Indeterminate(_))
    ));
}
