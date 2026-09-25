// SPDX-License-Identifier: MPL-2.0

//! Private native endpoint publications and bounded discovery. Native peer
//! authentication and exact-record stale retirement remain separate operations;
//! this foundation does not enable persistent-session frontend availability.

use super::windows_process_identity::ProcessIdentity;
use crate::{
    native_path::{decode_path, encode_path},
    private_storage::Directory,
};
use std::{
    ffi::OsStr,
    fs::File,
    io::{self, Read},
    path::{Path, PathBuf},
    sync::Arc,
};

mod discovery;
mod locking;
mod metadata;
mod names;
mod view;
pub use discovery::{
    AuthenticatedHost, Candidate, CandidateOrigin, Inspection, ProbeFailure, ProbedCandidate,
    ProbedScan, Removal, Scan, ScanIssue, ScanLimit, StaleEvidence, StaleReason,
};
pub use metadata::{
    EndpointMetadata, MAX_METADATA_BYTES, MAX_PERSISTED_PATH_BYTES, PipeAddress, RegistryRecord,
};
pub(crate) use names::StoppedNameSelection;
pub use names::{NameStore, StoppedNameCommit, StoppedNameEdit};
pub use view::RegistryView;

const REGISTRY_LOCK: &str = ".registry.lock";
const READY_NAME: &str = "endpoint.json";
const MAX_REGISTRIES: usize = 3;

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

/// Excludes publication startup from a project-wide operation such as Git
/// worktree removal. Every configured namespace for the same account uses the
/// owner inventory, so this lock is keyed only by the canonical project root.
/// Hold it until publication has either succeeded or failed; a remover holds
/// it while it checks and stops hosts and removes the worktree.
#[derive(Clone, Debug)]
pub struct ProjectLease(Arc<ProjectLeaseInner>);

#[derive(Debug)]
struct ProjectLeaseInner {
    project: PathBuf,
    project_identity: crate::windows_fs::Identity,
    inventory: PathBuf,
    inventory_key: locking::FileKey,
    lock_name: String,
    lock_key: locking::FileKey,
    _directory: Directory,
    _registry_file: File,
    _lock: locking::Lock,
}

impl ProjectLease {
    pub fn acquire(project: &Path, inventory: &Path) -> io::Result<Self> {
        let project = super::WorkspaceIdentity::resolve(project)?
            .root()
            .to_owned();
        let project_identity = crate::windows_fs::Identity::read(&project)?;
        metadata::persisted_path(&encode_path(&project))?;
        let id = crate::workspace::workspace_id(&project);
        let lock_name = format!(".project-{id}.lock");
        metadata::persisted_path(&encode_path(&inventory.join(&lock_name)))?;
        let directory = Directory::open(inventory, true)?;
        let lock_file = directory.append(OsStr::new(&lock_name))?;
        let lock_key = locking::file_key(&lock_file)?;
        let lock = locking::Lock::acquire(lock_file)?;
        let registry_file = directory.append(OsStr::new(REGISTRY_LOCK))?;
        let inventory_key = locking::file_key(&registry_file)?;
        let lease = Self(Arc::new(ProjectLeaseInner {
            project,
            project_identity,
            inventory: inventory.to_owned(),
            inventory_key,
            lock_name,
            lock_key,
            _directory: directory,
            _registry_file: registry_file,
            _lock: lock,
        }));
        // A remover may finish between the initial canonicalization and lock
        // acquisition. Refuse a missing or replaced root and any relocated
        // inventory lock before startup changes state.
        lease.verify_live_identity()?;
        Ok(lease)
    }

    pub fn project_root(&self) -> &Path {
        &self.0.project
    }

    pub(crate) fn project_identity_bytes(&self) -> [u8; 24] {
        self.0.project_identity.stable_bytes()
    }

    pub fn verify_live_identity(&self) -> io::Result<()> {
        if crate::windows_fs::Identity::read(&self.0.project)? != self.0.project_identity {
            return Err(invalid("project root changed while acquiring its lease"));
        }
        let directory = Directory::open_existing(&self.0.inventory, true)?;
        let registry = directory.open_read(OsStr::new(REGISTRY_LOCK))?;
        let project_lock = directory.open_read(OsStr::new(&self.0.lock_name))?;
        if locking::file_key(&registry)? != self.0.inventory_key
            || locking::file_key(&project_lock)? != self.0.lock_key
        {
            return Err(invalid("project lease inventory identity changed"));
        }
        Ok(())
    }

    fn covers(&self, project: &Path, registries: &RegistrySet) -> bool {
        self.0.project == project
            && registries
                .lease_root()
                .is_ok_and(|root| root.key == self.0.inventory_key)
    }
}

#[derive(Debug)]
struct RegistryRoot {
    directory: Arc<Directory>,
    path: PathBuf,
    key: locking::FileKey,
    inventory: bool,
    #[cfg(test)]
    fixture_lease: bool,
}

/// Explicit roots only. Normal namespaces may supply a primary, secondary and
/// owner-wide inventory root; tests must supply fixture-owned directories.
#[derive(Clone, Debug)]
pub struct RegistrySet(Arc<Vec<RegistryRoot>>);

impl RegistrySet {
    pub fn open(paths: &[PathBuf]) -> io::Result<Self> {
        Self::with_inventory(paths, None)
    }

    /// Tests without an owner inventory explicitly use their private fixture
    /// namespace for lease coverage. Production publication requires inventory.
    #[cfg(test)]
    pub(crate) fn open_fixture(paths: &[PathBuf]) -> io::Result<Self> {
        let mut set = Self::open(paths)?;
        Arc::get_mut(&mut set.0)
            .and_then(|roots| roots.first_mut())
            .expect("new fixture registry set has a root")
            .fixture_lease = true;
        Ok(set)
    }

    pub fn with_inventory(paths: &[PathBuf], inventory: Option<PathBuf>) -> io::Result<Self> {
        if paths.is_empty() || paths.len() + usize::from(inventory.is_some()) > MAX_REGISTRIES {
            return Err(invalid(
                "publication requires namespace roots and at most three total registry roots",
            ));
        }
        let roots = paths
            .iter()
            .map(|path| (path, false))
            .chain(inventory.as_ref().map(|path| (path, true)))
            .map(|(path, inventory)| {
                metadata::persisted_path(&encode_path(path))?;
                let directory = Arc::new(Directory::open(path, true)?);
                let key = locking::file_key(&directory.append(OsStr::new(REGISTRY_LOCK))?)?;
                Ok(RegistryRoot {
                    directory,
                    path: path.clone(),
                    key,
                    inventory,
                    #[cfg(test)]
                    fixture_lease: false,
                })
            })
            .collect::<io::Result<Vec<_>>>()?;
        Self::from_roots(roots)
    }

    fn from_roots(mut roots: Vec<RegistryRoot>) -> io::Result<Self> {
        roots.sort_by_key(|root| root.key);
        if roots
            .windows(2)
            .any(|pair| pair[0].key == pair[1].key && pair[0].inventory != pair[1].inventory)
        {
            return Err(invalid(
                "one registry directory cannot be both namespace and inventory",
            ));
        }
        roots.dedup_by_key(|root| root.key);
        Ok(Self(Arc::new(roots)))
    }

    fn identity_locks(&self, id: &str) -> io::Result<Vec<locking::Lock>> {
        metadata::validate_id(id)?;
        locking::acquire(&self.0, |root| {
            format!(".host-{}.lock", self.record_key(root, id))
        })
    }

    fn lease_root(&self) -> io::Result<&RegistryRoot> {
        self.0
            .iter()
            .find(|root| root.inventory)
            .or({
                #[cfg(test)]
                {
                    self.0.iter().find(|root| root.fixture_lease)
                }
                #[cfg(not(test))]
                {
                    None
                }
            })
            .ok_or_else(|| invalid("publication requires an admitted owner inventory"))
    }

    fn registry_locks(&self) -> io::Result<Vec<locking::Lock>> {
        locking::acquire(&self.0, |_| REGISTRY_LOCK.to_owned())
    }

    fn record_key(&self, root: &RegistryRoot, id: &str) -> String {
        if !root.inventory {
            return id.to_owned();
        }
        // Stable lock-file identities survive publication replacement and keep
        // deliberately isolated namespaces distinct in the owner-wide index.
        let mut bytes = b"runyte-windows-namespace-v1\0".to_vec();
        for namespace in self.0.iter().filter(|root| !root.inventory) {
            bytes.extend_from_slice(&namespace.key.0.to_le_bytes());
            bytes.extend_from_slice(&namespace.key.1);
        }
        format!("{id}-{}", &crate::hash::sha256_hex(&bytes)[..32])
    }

    /// Returns unverified candidates. Callers must authenticate the actual pipe
    /// peer before treating a candidate as a live host or targeting its process.
    pub fn read(&self, id: &str) -> io::Result<Vec<RegistryRecord>> {
        metadata::validate_id(id)?;
        let mut records = Vec::new();
        for root in self.0.iter() {
            let name = format!("{}.json", self.record_key(root, id));
            if let Some((_, bytes)) = read_optional(&root.directory, &name)? {
                let record = RegistryRecord::from_json(&bytes)?;
                if record.host.id != id {
                    return Err(invalid("registry workspace identity mismatch"));
                }
                records.push(record);
            }
        }
        Ok(records)
    }
}

/// Filesystem publication location, distinct from the generated pipe address.
#[derive(Clone, Debug)]
pub struct EndpointLocation {
    project: PathBuf,
    directory: PathBuf,
    registries: RegistrySet,
}

impl EndpointLocation {
    pub fn new(project: &Path, directory: PathBuf, registries: RegistrySet) -> io::Result<Self> {
        let project = super::WorkspaceIdentity::resolve(project)?
            .root()
            .to_owned();
        metadata::persisted_path(&encode_path(&project))?;
        metadata::persisted_path(&encode_path(&directory.join(READY_NAME)))?;
        Ok(Self {
            project,
            directory,
            registries,
        })
    }

    pub fn ready_record(&self) -> PathBuf {
        self.directory.join(READY_NAME)
    }

    pub fn project_root(&self) -> &Path {
        &self.project
    }

    pub(crate) fn verify_project_lease(&self, lease: &ProjectLease) -> io::Result<()> {
        lease.verify_live_identity()?;
        if !lease.covers(&self.project, &self.registries) {
            return Err(invalid(
                "project lease does not match the admitted inventory identity",
            ));
        }
        Ok(())
    }

    /// Exact configured keys, including this namespace's inventory entry.
    /// Does not enumerate or infer a hidden host's namespace.
    pub fn observe_registrations(&self) -> io::Result<Scan> {
        self.registries
            .observe(&crate::workspace::workspace_id(&self.project))
    }

    /// Acquire the project lease before identity and registry locks. All three
    /// classes remain held through bind and publication.
    pub fn prepare(&self, name: Option<String>) -> io::Result<PreparedEndpoint> {
        let lease = ProjectLease::acquire(&self.project, &self.registries.lease_root()?.path)?;
        self.prepare_with_lease(&lease, name)
    }

    /// Reuse a parent or host lease already held before other startup state
    /// changes. The prepared endpoint retains a shared guard through publish.
    pub fn prepare_with_lease(
        &self,
        lease: &ProjectLease,
        name: Option<String>,
    ) -> io::Result<PreparedEndpoint> {
        self.verify_project_lease(lease)?;
        let metadata = EndpointMetadata::new(&self.project, name)?;
        let mut locks = self.registries.identity_locks(&metadata.id)?;
        locks.extend(self.registries.registry_locks()?);
        if let Some(name) = &metadata.name {
            names::ensure_available(&self.registries, &metadata.id, name)?;
        }
        for root in self.registries.0.iter() {
            let registry_name = format!("{}.json", self.registries.record_key(root, &metadata.id));
            require_absent(&root.directory, &registry_name)?;
        }
        let directory = Arc::new(Directory::open(&self.directory, true)?);
        require_absent(&directory, READY_NAME)?;
        Ok(PreparedEndpoint {
            location: self.clone(),
            directory,
            metadata,
            project_lease: lease.clone(),
            _locks: locks,
        })
    }

    /// Read-only discovery, with no stale-record deletion or process action.
    pub fn read_ready(&self) -> io::Result<Option<EndpointMetadata>> {
        let directory = match Directory::open_existing(&self.directory, true) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            result => result?,
        };
        let Some((_, bytes)) = read_optional(&directory, READY_NAME)? else {
            return Ok(None);
        };
        let metadata = EndpointMetadata::from_json(&bytes)?;
        if metadata.project_root_bytes != encode_path(&self.project) {
            return Err(invalid("ready record workspace identity mismatch"));
        }
        Ok(Some(metadata))
    }
}

#[derive(Debug)]
pub struct PreparedEndpoint {
    location: EndpointLocation,
    directory: Arc<Directory>,
    metadata: EndpointMetadata,
    _locks: Vec<locking::Lock>,
    project_lease: ProjectLease,
}

impl PreparedEndpoint {
    pub fn metadata(&self) -> &EndpointMetadata {
        &self.metadata
    }

    /// The transport owner must bind the private listener before calling this.
    /// This module only publishes metadata; it cannot prove listener readiness.
    /// Registry records precede endpoint.json. Rollback errors can leave
    /// unverified residue and are reported alongside the original failure;
    /// residue never authorizes PID-only recovery. Names are not allocated here.
    pub fn publish(self) -> io::Result<Publication> {
        self.publish_before_ready(|| Ok(()))
    }

    fn publish_before_ready(
        self,
        before_ready: impl FnOnce() -> io::Result<()>,
    ) -> io::Result<Publication> {
        self.location.verify_project_lease(&self.project_lease)?;
        let mut issued = Vec::new();
        let record = RegistryRecord {
            host: self.metadata.clone(),
            ready_record_bytes: encode_path(&self.location.ready_record()),
        };
        let registry_bytes = record.to_json()?;
        let ready_bytes = self.metadata.to_json()?;
        let result: io::Result<()> = (|| {
            for root in self.location.registries.0.iter() {
                let registry_name = format!(
                    "{}.json",
                    self.location.registries.record_key(root, &self.metadata.id)
                );
                require_absent(&root.directory, &registry_name)?;
                let file = root
                    .directory
                    .atomic_write_owned(OsStr::new(&registry_name), &registry_bytes)?;
                issued.push(Issued {
                    directory: root.directory.clone(),
                    path: root.path.join(&registry_name),
                    name: registry_name.clone(),
                    file,
                    bytes: registry_bytes.clone(),
                    registry: true,
                });
                issued.last().unwrap().verify_path()?;
            }
            before_ready()?;
            // Recheck the whole set at the readiness boundary: a prior row can
            // have been replaced after its individual installation was checked.
            for record in &issued {
                record.verify_path()?;
            }
            require_absent(&self.directory, READY_NAME)?;
            let file = self
                .directory
                .atomic_write_owned(OsStr::new(READY_NAME), &ready_bytes)?;
            issued.push(Issued {
                directory: self.directory.clone(),
                path: self.location.ready_record(),
                name: READY_NAME.to_owned(),
                file,
                bytes: ready_bytes.clone(),
                registry: false,
            });
            // The advertised pathname must still resolve to the exact issued
            // ready record, even if an endpoint ancestor was renamed meanwhile.
            for record in &issued {
                record.verify_path()?;
            }
            Ok(())
        })();
        if let Err(error) = result {
            // Existing guards still cover rollback. Never reacquire our own
            // exclusive locks and never remove a replacement's identity.
            let mut cleanup_error = None;
            for record in issued.iter().rev() {
                if let Err(cleanup) = record.remove_if_matches(&self.metadata.incarnation) {
                    cleanup_error.get_or_insert(cleanup);
                }
            }
            if let Some(cleanup) = cleanup_error {
                return Err(io::Error::new(
                    error.kind(),
                    format!("{error}; publication rollback left unverified residue: {cleanup}"),
                ));
            }
            return Err(error);
        }
        Ok(Publication {
            location: self.location.clone(),
            metadata: self.metadata.clone(),
            issued,
            rename_recovery: None,
            retiring: false,
        })
    }
}

#[derive(Debug)]
struct Issued {
    directory: Arc<Directory>,
    path: PathBuf,
    name: String,
    file: File,
    bytes: Vec<u8>,
    registry: bool,
}

impl Issued {
    fn verify_path(&self) -> io::Result<()> {
        let parent = self
            .path
            .parent()
            .ok_or_else(|| invalid("publication path has no parent"))?;
        let directory = Directory::open_existing(parent, true)?;
        let (current, bytes) = read_optional(&directory, &self.name)?.ok_or_else(|| {
            io::Error::new(io::ErrorKind::NotFound, "published record path is missing")
        })?;
        if locking::file_key(&current)? != locking::file_key(&self.file)? || bytes != self.bytes {
            return Err(invalid("publication path changed identity or contents"));
        }
        Ok(())
    }

    fn remove_if_matches(&self, incarnation: &str) -> io::Result<()> {
        let Some((current, bytes)) = read_optional(&self.directory, &self.name)? else {
            return Ok(());
        };
        if locking::file_key(&current)? != locking::file_key(&self.file)? {
            return Ok(());
        }
        let actual = if self.registry {
            RegistryRecord::from_json(&bytes)?.host.incarnation
        } else {
            EndpointMetadata::from_json(&bytes)?.incarnation
        };
        if actual == incarnation {
            self.directory
                .remove_owned(OsStr::new(&self.name), &self.file)?;
            self.directory.sync()?;
        }
        Ok(())
    }
}

/// Owns publication identities, not the listener or host process. Cleanup is
/// explicit and retryable. Drop is best effort: contention leaves records intact
/// for later authenticated lifecycle recovery, never PID-only stale deletion.
#[derive(Debug)]
pub struct Publication {
    location: EndpointLocation,
    metadata: EndpointMetadata,
    issued: Vec<Issued>,
    rename_recovery: Option<names::RenameRecovery>,
    retiring: bool,
}

impl Publication {
    pub fn metadata(&self) -> &EndpointMetadata {
        &self.metadata
    }

    pub fn cleanup(&mut self) -> io::Result<()> {
        self.retiring = true;
        if self.issued.is_empty() && self.rename_recovery.is_none() {
            return Ok(());
        }
        let _identity = self.location.registries.identity_locks(&self.metadata.id)?;
        let _registry = self.location.registries.registry_locks()?;
        let mut failure = self.cleanup_rename_recovery().err();
        // The ready record is last published and first retired.
        for record in self.issued.iter().rev() {
            if let Err(error) = record.remove_if_matches(&self.metadata.incarnation) {
                failure.get_or_insert(error);
            }
        }
        if let Some(error) = failure {
            return Err(error);
        }
        self.issued.clear();
        Ok(())
    }
}

impl Drop for Publication {
    fn drop(&mut self) {
        let _ = self.cleanup();
    }
}

fn require_absent(directory: &Directory, name: &str) -> io::Result<()> {
    match directory.open_read(OsStr::new(name)) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
        Ok(_) => Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "workspace publication is occupied and has not been verified stale",
        )),
    }
}

/// Resolves account storage independently of inherited workspace/XDG settings.
/// Resolution alone creates nothing; RegistrySet admission verifies private NTFS.
pub fn inventory_root() -> io::Result<PathBuf> {
    crate::user_paths::system_local_app_data_directory()
        .map(|path| inventory_root_in(&path))
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                "account local application-data directory is unavailable",
            )
        })
}

pub(crate) fn inventory_root_in(local_app_data: &Path) -> PathBuf {
    local_app_data.join("runyte").join("all-hosts")
}

fn read_optional(directory: &Directory, name: &str) -> io::Result<Option<(File, Vec<u8>)>> {
    let mut file = match directory.open_read(OsStr::new(name)) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        result => result?,
    };
    let mut bytes = Vec::new();
    (&mut file)
        .take((MAX_METADATA_BYTES + 1) as u64)
        .read_to_end(&mut bytes)?;
    if bytes.len() > MAX_METADATA_BYTES {
        return Err(invalid("host metadata exceeds 64 KiB"));
    }
    Ok(Some((file, bytes)))
}

#[cfg(test)]
mod tests;
