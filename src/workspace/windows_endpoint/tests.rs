// SPDX-License-Identifier: MPL-2.0

use super::*;
use crate::test_support::TestRuntimeRoot;
use std::{
    fs,
    io::Write,
    os::windows::{ffi::OsStringExt, io::AsRawHandle, process::CommandExt},
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::WAIT_OBJECT_0,
    System::Threading::{CREATE_NO_WINDOW, WaitForSingleObject},
};

fn location(root: &Path) -> EndpointLocation {
    let project = root.join("project");
    fs::create_dir_all(&project).unwrap();
    let registries = RegistrySet::open(&[root.join("registry")]).unwrap();
    EndpointLocation::new(&project, root.join("endpoint"), registries).unwrap()
}

#[test]
fn metadata_separates_pipe_address_and_filesystem_record_with_exact_native_paths() {
    let project = PathBuf::from(std::ffi::OsString::from_wide(&[
        67, 58, 92, 0xd800, 0x00e9, 0xd83d, 0xde00,
    ]));
    let metadata = EndpointMetadata::new(&project, Some("editor".to_owned())).unwrap();
    assert_eq!(metadata.project_root().unwrap(), project);
    assert_eq!(
        EndpointMetadata::from_json(&metadata.to_json().unwrap()).unwrap(),
        metadata
    );
    let second = EndpointMetadata::new(&project, None).unwrap();
    assert_ne!(metadata.incarnation, second.incarnation);
    assert_ne!(metadata.address, second.address);
    assert!(
        metadata
            .address
            .as_str()
            .starts_with(r"\\.\pipe\runyte-v1-")
    );
    let record = RegistryRecord {
        host: metadata,
        ready_record_bytes: encode_path(Path::new(r"C:\private\endpoint.json")),
    };
    assert_eq!(
        RegistryRecord::from_json(&record.to_json().unwrap()).unwrap(),
        record
    );
    assert_eq!(
        record.ready_record().unwrap(),
        Path::new(r"C:\private\endpoint.json")
    );
    assert_eq!(
        inventory_root_in(Path::new(r"D:\account")),
        Path::new(r"D:\account\runyte\hosts-v1")
    );
}

#[test]
fn malformed_metadata_refuses_bad_paths_addresses_identity_and_size() {
    let valid = EndpointMetadata::new(Path::new(r"C:\project"), None).unwrap();
    for path in [
        Vec::new(),
        vec![65],
        encode_path(Path::new("relative")),
        vec![65; MAX_PERSISTED_PATH_BYTES + 2],
        encode_path(Path::new("C:\\bad\0name")),
    ] {
        let mut candidate = valid.clone();
        candidate.project_root_bytes = path;
        assert!(EndpointMetadata::from_json(&serde_json::to_vec(&candidate).unwrap()).is_err());
    }
    for field in ["id", "incarnation", "address"] {
        let mut value = serde_json::to_value(&valid).unwrap();
        value[field] = serde_json::Value::String("invalid".to_owned());
        assert!(EndpointMetadata::from_json(&serde_json::to_vec(&value).unwrap()).is_err());
    }
    for address in [
        r"\\server\pipe\runyte-v1-",
        r"C:\pipe",
        r"\\.\pipe\runyte-v1-..\other",
        r"\\?\pipe\runyte-v1-",
    ] {
        assert!(PipeAddress::try_from(address.to_owned()).is_err());
    }
    for name in ["", " spaced", "line\nfeed", &"x".repeat(65)] {
        let mut candidate = valid.clone();
        candidate.name = Some(name.to_owned());
        assert!(candidate.validate().is_err());
    }
    let mut value = serde_json::to_value(&valid).unwrap();
    value["process"]["creation_time"] = 0.into();
    assert!(EndpointMetadata::from_json(&serde_json::to_vec(&value).unwrap()).is_err());
    value = serde_json::to_value(&valid).unwrap();
    value["unexpected"] = true.into();
    assert!(EndpointMetadata::from_json(&serde_json::to_vec(&value).unwrap()).is_err());
    assert!(EndpointMetadata::from_json(&vec![b' '; MAX_METADATA_BYTES + 1]).is_err());
    let bad_record = RegistryRecord {
        host: valid,
        ready_record_bytes: vec![65],
    };
    assert!(RegistryRecord::from_json(&serde_json::to_vec(&bad_record).unwrap()).is_err());
}

#[test]
fn separate_handles_contend_release_and_keep_stable_lock_files() {
    let root = TestRuntimeRoot::new("endpoint-locks").unwrap();
    let registry = root.join("registry");
    let first = RegistrySet::open(std::slice::from_ref(&registry)).unwrap();
    let alias = registry.join(".");
    let second = RegistrySet::open(&[alias, registry.clone()]).unwrap();
    assert_eq!(second.0.len(), 1);
    let id = "a".repeat(32);
    let guards = first.identity_locks(&id).unwrap();
    assert_eq!(
        second.identity_locks(&id).unwrap_err().kind(),
        io::ErrorKind::WouldBlock
    );
    let lock_path = registry.join(format!(".host-{id}.lock"));
    let before = crate::windows_fs::Identity::read(&lock_path).unwrap();
    drop(guards);
    drop(second.identity_locks(&id).unwrap());
    assert_eq!(
        crate::windows_fs::Identity::read(&lock_path).unwrap(),
        before
    );
}

#[test]
fn shared_secondary_registry_serializes_different_primary_roots_before_endpoint_preparation() {
    let root = TestRuntimeRoot::new("endpoint-shared").unwrap();
    let first_project = root.create_private_dir("first-project").unwrap();
    let second_project = root.create_private_dir("second-project").unwrap();
    let common = root.join("shared");
    let first = EndpointLocation::new(
        &first_project,
        root.join("first-endpoint"),
        RegistrySet::open(&[root.join("first"), common.clone()]).unwrap(),
    )
    .unwrap();
    let second = EndpointLocation::new(
        &second_project,
        root.join("second-endpoint"),
        RegistrySet::open(&[common, root.join("second")]).unwrap(),
    )
    .unwrap();
    let held = first.prepare(None).unwrap();
    assert_eq!(
        second.prepare(None).unwrap_err().kind(),
        io::ErrorKind::WouldBlock
    );
    assert!(!second.directory.try_exists().unwrap());
    drop(held);
    drop(second.prepare(None).unwrap());
}

#[test]
fn isolated_namespaces_can_publish_the_same_project_in_one_owner_inventory() {
    let root = TestRuntimeRoot::new("endpoint-namespaces").unwrap();
    let project = root.create_private_dir("project").unwrap();
    let inventory = root.join("inventory");
    let first = EndpointLocation::new(
        &project,
        root.join("first-endpoint"),
        RegistrySet::with_inventory(&[root.join("first")], Some(inventory.clone())).unwrap(),
    )
    .unwrap();
    let second = EndpointLocation::new(
        &project,
        root.join("second-endpoint"),
        RegistrySet::with_inventory(&[root.join("second")], Some(inventory.clone())).unwrap(),
    )
    .unwrap();
    let mut first_publication = first.prepare(None).unwrap().publish().unwrap();
    let mut second_publication = second.prepare(None).unwrap().publish().unwrap();
    assert_eq!(
        first_publication.metadata.id,
        second_publication.metadata.id
    );
    assert_ne!(
        first_publication.metadata.incarnation,
        second_publication.metadata.incarnation
    );
    let id = first_publication.metadata.id.clone();
    assert_eq!(first.registries.read(&id).unwrap().len(), 2);
    assert_eq!(second.registries.read(&id).unwrap().len(), 2);
    first_publication.cleanup().unwrap();
    assert!(first.registries.read(&id).unwrap().is_empty());
    assert_eq!(second.registries.read(&id).unwrap().len(), 2);
    assert!(second.read_ready().unwrap().is_some());
    second_publication.cleanup().unwrap();
    assert!(
        RegistrySet::with_inventory(std::slice::from_ref(&inventory), Some(inventory.clone()))
            .is_err()
    );
}

#[test]
fn distinct_lock_sets_cannot_replace_one_ready_record() {
    let root = TestRuntimeRoot::new("endpoint-no-clobber").unwrap();
    let project = root.create_private_dir("project").unwrap();
    let directory = root.join("endpoint");
    let first = EndpointLocation::new(
        &project,
        directory.clone(),
        RegistrySet::open(&[root.join("first")]).unwrap(),
    )
    .unwrap();
    let second = EndpointLocation::new(
        &project,
        directory,
        RegistrySet::open(&[root.join("second")]).unwrap(),
    )
    .unwrap();
    let prepared_first = first.prepare(None).unwrap();
    let prepared_second = second.prepare(None).unwrap();
    let first_publication = prepared_first.publish().unwrap();
    assert_eq!(
        prepared_second.publish().unwrap_err().kind(),
        io::ErrorKind::AlreadyExists
    );
    assert_eq!(
        first.read_ready().unwrap().as_ref(),
        Some(first_publication.metadata())
    );
    assert!(
        second
            .registries
            .read(&first_publication.metadata.id)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn replaced_registry_path_cannot_publish_ready_into_an_undiscoverable_namespace() {
    let root = TestRuntimeRoot::new("endpoint-root-replaced").unwrap();
    let endpoint = location(root.path());
    let registry = root.join("registry");
    let moved = root.join("retired-registry");
    // No child handles are retained yet, so Windows permits the parent rename.
    fs::rename(&registry, &moved).unwrap();
    Directory::open(&registry, true).unwrap();
    let error = endpoint.prepare(None).unwrap().publish().unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::NotFound);
    assert!(endpoint.read_ready().unwrap().is_none());
    let id = crate::workspace::workspace_id(&endpoint.project);
    assert!(!moved.join(format!("{id}.json")).try_exists().unwrap());
    assert!(!registry.join(format!("{id}.json")).try_exists().unwrap());
}

#[test]
fn failed_publication_reports_cleanup_error_and_attempts_remaining_issued_records() {
    let root = TestRuntimeRoot::new("endpoint-rollback").unwrap();
    let project = root.create_private_dir("project").unwrap();
    let registries = RegistrySet::open(&[root.join("first"), root.join("second")]).unwrap();
    let endpoint = EndpointLocation::new(&project, root.join("endpoint"), registries).unwrap();
    let prepared = endpoint.prepare(None).unwrap();
    let name = format!("{}.json", prepared.metadata.id);
    // Cleanup visits the most recently issued record first. Make only that
    // record inadmissible, proving its failure cannot skip the remaining one.
    let damaged_path = endpoint.registries.0.last().unwrap().path.join(&name);
    let other_path = endpoint.registries.0.first().unwrap().path.join(&name);
    let error = prepared
        .publish_before_ready(|| {
            fs::hard_link(&damaged_path, root.join("extra-link"))?;
            Err(io::Error::other("original publication failure"))
        })
        .unwrap_err();
    assert!(error.to_string().contains("original publication failure"));
    assert!(error.to_string().contains("unverified residue"));
    assert!(damaged_path.try_exists().unwrap());
    assert!(!other_path.try_exists().unwrap());
    assert!(endpoint.read_ready().unwrap().is_none());
}

#[test]
fn registry_replacement_at_readiness_boundary_is_preserved_without_publishing_ready() {
    let root = TestRuntimeRoot::new("endpoint-boundary-replaced").unwrap();
    let project = root.create_private_dir("project").unwrap();
    let registries = RegistrySet::open(&[root.join("first"), root.join("second")]).unwrap();
    let endpoint = EndpointLocation::new(&project, root.join("endpoint"), registries).unwrap();
    let prepared = endpoint.prepare(None).unwrap();
    let name = format!("{}.json", prepared.metadata.id);
    let replacement = RegistryRecord {
        host: EndpointMetadata::new(&endpoint.project, None).unwrap(),
        ready_record_bytes: encode_path(&endpoint.ready_record()),
    };
    let replaced = &endpoint.registries.0[0];
    let untouched = &endpoint.registries.0[1];
    let error = prepared
        .publish_before_ready(|| {
            replaced
                .directory
                .atomic_write(OsStr::new(&name), &replacement.to_json()?)
        })
        .unwrap_err();
    assert!(error.to_string().contains("changed identity or contents"));
    assert!(endpoint.read_ready().unwrap().is_none());
    assert_eq!(
        RegistryRecord::from_json(
            &replaced
                .directory
                .read(OsStr::new(&name), MAX_METADATA_BYTES)
                .unwrap()
        )
        .unwrap(),
        replacement
    );
    assert!(
        read_optional(&untouched.directory, &name)
            .unwrap()
            .is_none()
    );
}

#[test]
fn registry_publication_precedes_ready_and_failure_rolls_back_issued_records() {
    let root = TestRuntimeRoot::new("endpoint-publish").unwrap();
    let endpoint = location(root.path());
    let id = crate::workspace::workspace_id(&endpoint.project);
    let prepared = endpoint.prepare(None).unwrap();
    assert!(endpoint.read_ready().unwrap().is_none());
    let error = prepared
        .publish_before_ready(|| {
            assert_eq!(endpoint.registries.read(&id).unwrap().len(), 1);
            assert!(endpoint.read_ready().unwrap().is_none());
            assert_eq!(
                endpoint.prepare(None).unwrap_err().kind(),
                io::ErrorKind::WouldBlock
            );
            Err(io::Error::other("injected publication failure"))
        })
        .unwrap_err();
    assert!(error.to_string().contains("injected publication"));
    assert!(endpoint.read_ready().unwrap().is_none());
    assert!(endpoint.registries.read(&id).unwrap().is_empty());
    let mut publication = endpoint
        .prepare(Some("fixture".to_owned()))
        .unwrap()
        .publish()
        .unwrap();
    assert_eq!(
        endpoint.read_ready().unwrap().as_ref(),
        Some(publication.metadata())
    );
    assert_eq!(
        endpoint.registries.read(&id).unwrap()[0].host,
        *publication.metadata()
    );
    assert_eq!(
        endpoint.prepare(None).unwrap_err().kind(),
        io::ErrorKind::AlreadyExists
    );
    let held = endpoint.registries.registry_locks().unwrap();
    assert_eq!(
        publication.cleanup().unwrap_err().kind(),
        io::ErrorKind::WouldBlock
    );
    assert!(endpoint.read_ready().unwrap().is_some());
    drop(held);
    publication.cleanup().unwrap();
    assert!(endpoint.read_ready().unwrap().is_none());
    assert!(endpoint.registries.read(&id).unwrap().is_empty());
}

#[test]
fn old_cleanup_preserves_replacement_files_even_when_they_copy_its_incarnation() {
    for copy_incarnation in [false, true] {
        let root = TestRuntimeRoot::new("endpoint-replaced").unwrap();
        let endpoint = location(root.path());
        let mut publication = endpoint.prepare(None).unwrap().publish().unwrap();
        let mut replacement =
            EndpointMetadata::new(&endpoint.project, Some("replacement".to_owned())).unwrap();
        if copy_incarnation {
            replacement.incarnation = publication.metadata.incarnation.clone();
        }
        for issued in &publication.issued {
            let bytes = if issued.registry {
                RegistryRecord {
                    host: replacement.clone(),
                    ready_record_bytes: encode_path(&endpoint.ready_record()),
                }
                .to_json()
                .unwrap()
            } else {
                replacement.to_json().unwrap()
            };
            issued
                .directory
                .atomic_write(OsStr::new(&issued.name), &bytes)
                .unwrap();
        }
        publication.cleanup().unwrap();
        assert_eq!(endpoint.read_ready().unwrap(), Some(replacement.clone()));
        assert_eq!(
            endpoint.registries.read(&replacement.id).unwrap()[0].host,
            replacement
        );
    }
}

#[test]
fn cleanup_rechecks_incarnation_even_if_file_identity_did_not_change() {
    use std::io::{Seek, SeekFrom};
    let root = TestRuntimeRoot::new("endpoint-incarnation").unwrap();
    let endpoint = location(root.path());
    let mut publication = endpoint.prepare(None).unwrap().publish().unwrap();
    let replacement = EndpointMetadata::new(&endpoint.project, None).unwrap();
    for issued in &publication.issued {
        let bytes = if issued.registry {
            RegistryRecord {
                host: replacement.clone(),
                ready_record_bytes: encode_path(&endpoint.ready_record()),
            }
            .to_json()
            .unwrap()
        } else {
            replacement.to_json().unwrap()
        };
        let mut file = issued.file.try_clone().unwrap();
        file.set_len(0).unwrap();
        file.seek(SeekFrom::Start(0)).unwrap();
        file.write_all(&bytes).unwrap();
        file.sync_all().unwrap();
    }
    publication.cleanup().unwrap();
    assert_eq!(endpoint.read_ready().unwrap(), Some(replacement));
}

#[test]
fn occupied_unverified_and_unsafe_records_are_never_removed_or_followed() {
    let root = TestRuntimeRoot::new("endpoint-refusal").unwrap();
    let endpoint = location(root.path());
    let prepared = endpoint.prepare(None).unwrap();
    drop(prepared);
    let record = endpoint.ready_record();
    let directory = Directory::open_existing(&endpoint.directory, true).unwrap();
    directory
        .atomic_write(OsStr::new(READY_NAME), b"unverified")
        .unwrap();
    assert_eq!(
        endpoint.prepare(None).unwrap_err().kind(),
        io::ErrorKind::AlreadyExists
    );
    assert!(endpoint.read_ready().is_err());
    assert_eq!(fs::read(&record).unwrap(), b"unverified");
    directory
        .atomic_write(OsStr::new(READY_NAME), &vec![b'x'; MAX_METADATA_BYTES + 1])
        .unwrap();
    assert!(endpoint.read_ready().is_err());
    assert_eq!(
        fs::metadata(&record).unwrap().len(),
        (MAX_METADATA_BYTES + 1) as u64
    );
    let linked = root.join("hardlinked-record");
    fs::hard_link(&record, &linked).unwrap();
    assert!(endpoint.read_ready().is_err());
    assert!(endpoint.prepare(None).is_err());
    assert!(RegistrySet::open(&[root.join("registry:stream")]).is_err());
    assert!(RegistrySet::open(&[PathBuf::from("relative")]).is_err());
    assert!(RegistrySet::open(&[PathBuf::from(r"\\server\share\registry")]).is_err());
}

#[test]
fn unverifiable_process_metadata_does_not_authorize_stale_removal() {
    let root = TestRuntimeRoot::new("endpoint-unverified-pid").unwrap();
    let endpoint = location(root.path());
    let prepared = endpoint.prepare(None).unwrap();
    let mut metadata = prepared.metadata.clone();
    metadata.process = ProcessIdentity {
        pid: u32::MAX,
        creation_time: 1,
    };
    prepared
        .directory
        .atomic_write(OsStr::new(READY_NAME), &metadata.to_json().unwrap())
        .unwrap();
    drop(prepared);
    assert_eq!(
        endpoint.prepare(None).unwrap_err().kind(),
        io::ErrorKind::AlreadyExists
    );
    assert_eq!(endpoint.read_ready().unwrap(), Some(metadata));
}

struct FixtureChild(Child);
impl FixtureChild {
    fn spawn(root: &Path, mode: &str) -> Self {
        Self(
            Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "workspace::windows_endpoint::tests::lock_fixture",
                    "--ignored",
                    "--nocapture",
                ])
                .env("RUNYTE_ENDPOINT_FIXTURE_ROOT", root)
                .env("RUNYTE_ENDPOINT_FIXTURE_MODE", mode)
                .env("XDG_CONFIG_HOME", root.join("config"))
                .env("XDG_CACHE_HOME", root.join("cache"))
                .creation_flags(CREATE_NO_WINDOW)
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap(),
        )
    }
    fn send(&mut self, byte: u8) {
        self.0.stdin.as_mut().unwrap().write_all(&[byte]).unwrap();
    }
    fn finished(&mut self) {
        assert_eq!(
            unsafe { WaitForSingleObject(self.0.as_raw_handle(), 10000) },
            WAIT_OBJECT_0
        );
        assert!(self.0.try_wait().unwrap().unwrap().success());
    }
}
impl Drop for FixtureChild {
    fn drop(&mut self) {
        let _ = self.0.kill();
        unsafe {
            WaitForSingleObject(self.0.as_raw_handle(), 5000);
        }
    }
}

fn wait_marker(path: &Path, expected: &[u8]) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if fs::read(path).is_ok_and(|bytes| bytes == expected) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "native lock fixture did not reach its gate"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn compiled_processes_contend_and_release_the_same_identity_lock() {
    let root = TestRuntimeRoot::new("endpoint-processes").unwrap();
    let _endpoint = location(root.path());
    let mut first = FixtureChild::spawn(root.path(), "hold");
    wait_marker(&root.join("holder"), b"held");
    let mut second = FixtureChild::spawn(root.path(), "contend");
    wait_marker(&root.join("contender"), b"busy");
    first.send(b'q');
    first.finished();
    second.send(b'r');
    wait_marker(&root.join("contender"), b"acquired");
    second.finished();
}

#[test]
#[ignore = "compiled native fixture for endpoint publication lock ownership"]
fn lock_fixture() {
    let root = PathBuf::from(std::env::var_os("RUNYTE_ENDPOINT_FIXTURE_ROOT").unwrap());
    let endpoint = location(&root);
    let mode = std::env::var("RUNYTE_ENDPOINT_FIXTURE_MODE").unwrap();
    if mode == "hold" {
        let held = endpoint.prepare(None).unwrap();
        fs::write(root.join("holder"), b"held").unwrap();
        let mut byte = [0];
        std::io::stdin().read_exact(&mut byte).unwrap();
        assert_eq!(byte, [b'q']);
        drop(held);
    } else {
        assert_eq!(mode, "contend");
        assert_eq!(
            endpoint.prepare(None).unwrap_err().kind(),
            io::ErrorKind::WouldBlock
        );
        fs::write(root.join("contender"), b"busy").unwrap();
        let mut byte = [0];
        std::io::stdin().read_exact(&mut byte).unwrap();
        assert_eq!(byte, [b'r']);
        let held = endpoint.prepare(None).unwrap();
        fs::write(root.join("contender"), b"acquired").unwrap();
        drop(held);
    }
}
