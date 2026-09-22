// SPDX-License-Identifier: MPL-2.0

//! Bounded observations, native peer proof, and exact-record stale retirement.
//! Filesystem work is synchronous and belongs off the editor loop when wired.

use super::*;
use crate::workspace::{
    windows_pipe,
    windows_process_identity::{PinResult, PinnedProcess},
};
use std::time::Duration;
use tokio::time::Instant;

const MAX_SCAN_ENTRIES: usize = 4096;
const MAX_SCAN_ROWS: usize = 1024;
const MAX_SCAN_BYTES: usize = 16 * 1024 * 1024;
const PROBE_BUDGET: Duration = Duration::from_millis(250);
const TOTAL_PROBE_BUDGET: Duration = Duration::from_secs(2);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CandidateOrigin {
    ConfiguredNamespace,
    OwnerInventory,
    ConfiguredReady,
}

#[derive(Debug)]
enum Source {
    Row {
        registries: RegistrySet,
        root: usize,
        inventory: bool,
    },
    Ready(EndpointLocation),
    UnscopedReady,
}

/// Metadata and an exact admitted file are observations, never process/control
/// authority. The retained file prevents identity reuse while cleanup is pending.
#[derive(Debug)]
pub struct Candidate {
    issued: Issued,
    source: Source,
    metadata: EndpointMetadata,
    record: Option<RegistryRecord>,
}

impl Candidate {
    pub fn metadata(&self) -> &EndpointMetadata {
        &self.metadata
    }
    pub fn registry_record(&self) -> Option<&RegistryRecord> {
        self.record.as_ref()
    }
    pub fn origin(&self) -> CandidateOrigin {
        match &self.source {
            Source::Row {
                inventory: true, ..
            } => CandidateOrigin::OwnerInventory,
            Source::Row {
                inventory: false, ..
            } => CandidateOrigin::ConfiguredNamespace,
            Source::Ready(_) | Source::UnscopedReady => CandidateOrigin::ConfiguredReady,
        }
    }

    /// A matching process is only potentially live. It is not an authenticated
    /// host until the actual pipe peer proves the same retained native identity.
    pub fn inspect_process(&self) -> io::Result<Inspection<'_>> {
        self.inspect_with(PinnedProcess::open)
    }

    fn inspect_with(
        &self,
        pin: impl FnOnce(ProcessIdentity) -> io::Result<PinResult>,
    ) -> io::Result<Inspection<'_>> {
        match pin(self.metadata.process)? {
            PinResult::Pinned(process) => Ok(Inspection::Present(Arc::new(process))),
            PinResult::Gone => Ok(Inspection::Stale(StaleEvidence {
                candidate: self,
                reason: StaleReason::Gone,
            })),
            PinResult::Reused => Ok(Inspection::Stale(StaleEvidence {
                candidate: self,
                reason: StaleReason::Reused,
            })),
        }
    }

    /// Authenticates native peer identity even for an incompatible protocol.
    /// No Hello/control operation is sent, and failure never authorizes cleanup.
    pub async fn authenticate(&self, deadline: Instant) -> io::Result<AuthenticatedHost> {
        let stream = windows_pipe::connect(&self.metadata, deadline).await?;
        Ok(AuthenticatedHost {
            metadata: self.metadata.clone(),
            peer: stream.peer().clone(),
        })
    }

    pub(super) fn disallow_unscoped_ready_cleanup(&mut self) {
        self.source = Source::UnscopedReady;
    }

    fn locks(&self) -> io::Result<Vec<locking::Lock>> {
        match &self.source {
            Source::UnscopedReady => Err(invalid(
                "ready retirement requires configured namespace identities",
            )),
            Source::Ready(location) => {
                let mut locks = location.registries.identity_locks(&self.metadata.id)?;
                locks.extend(location.registries.registry_locks()?);
                Ok(locks)
            }
            Source::Row {
                registries,
                inventory: false,
                ..
            } => {
                let mut locks = registries.identity_locks(&self.metadata.id)?;
                locks.extend(registries.registry_locks()?);
                Ok(locks)
            }
            Source::Row {
                registries,
                root,
                inventory: true,
            } => {
                // The observed inventory row supplies only its own lock key.
                // Its reported ready path never reconstructs namespace roots.
                let root = std::slice::from_ref(&registries.0[*root]);
                let stem = self
                    .issued
                    .name
                    .strip_suffix(".json")
                    .expect("admitted registry filename");
                let mut locks = locking::acquire(root, |_| format!(".host-{stem}.lock"))?;
                locks.extend(locking::acquire(root, |_| REGISTRY_LOCK.to_owned())?);
                Ok(locks)
            }
        }
    }
}

#[derive(Debug)]
pub enum Inspection<'a> {
    Present(Arc<PinnedProcess>),
    Stale(StaleEvidence<'a>),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StaleReason {
    Gone,
    Reused,
}

/// Only conclusive process inspection constructs this evidence. Cleanup still
/// repeats the proof and exact file/bytes/incarnation checks under known locks.
#[derive(Debug)]
pub struct StaleEvidence<'a> {
    candidate: &'a Candidate,
    reason: StaleReason,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Removal {
    Removed,
    Missing,
    Changed,
    ProcessPresent,
}

impl StaleEvidence<'_> {
    pub fn reason(&self) -> StaleReason {
        self.reason
    }
    pub fn remove_observed(self) -> io::Result<Removal> {
        self.remove_with(PinnedProcess::open)
    }

    fn remove_with(
        self,
        pin: impl FnOnce(ProcessIdentity) -> io::Result<PinResult>,
    ) -> io::Result<Removal> {
        let candidate = self.candidate;
        let _locks = candidate.locks()?;
        if matches!(pin(candidate.metadata.process)?, PinResult::Pinned(_)) {
            return Ok(Removal::ProcessPresent);
        }
        let Some((file, bytes)) =
            read_optional(&candidate.issued.directory, &candidate.issued.name)?
        else {
            return Ok(Removal::Missing);
        };
        if locking::file_key(&file)? != locking::file_key(&candidate.issued.file)?
            || bytes != candidate.issued.bytes
        {
            return Ok(Removal::Changed);
        }
        let current = if candidate.record.is_some() {
            RegistryRecord::from_json(&bytes)?.host
        } else {
            EndpointMetadata::from_json(&bytes)?
        };
        if current.incarnation != candidate.metadata.incarnation || current != candidate.metadata {
            return Ok(Removal::Changed);
        }
        // A relocated pinned root must not masquerade as its old namespace.
        candidate.issued.verify_path()?;
        candidate
            .issued
            .directory
            .remove_owned(OsStr::new(&candidate.issued.name), &candidate.issued.file)?;
        candidate.issued.directory.sync()?;
        Ok(Removal::Removed)
    }
}

/// Native peer proof only. Compatible control operations still require Hello
/// and must authenticate their own connection; a prior probe is not a session.
#[derive(Debug)]
pub struct AuthenticatedHost {
    metadata: EndpointMetadata,
    peer: Arc<PinnedProcess>,
}

impl AuthenticatedHost {
    pub fn metadata(&self) -> &EndpointMetadata {
        &self.metadata
    }
    pub fn peer(&self) -> &Arc<PinnedProcess> {
        &self.peer
    }
    pub fn speaks_current_protocol(&self) -> bool {
        self.metadata.protocol == crate::protocol::VERSION
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScanLimit {
    Entries,
    Rows,
    MetadataBytes,
}

#[derive(Debug)]
pub struct ScanIssue {
    pub root: PathBuf,
    pub name: Option<String>,
    pub error: io::Error,
}

#[derive(Debug, Default)]
pub struct Scan {
    pub candidates: Vec<Candidate>,
    pub issues: Vec<ScanIssue>,
    /// A bounded scan never implies undiscovered hosts are absent.
    pub limit: Option<ScanLimit>,
}

#[derive(Debug)]
pub enum ProbeFailure {
    BudgetExhausted,
    Indeterminate(io::Error),
}

#[derive(Debug)]
pub struct ProbedCandidate {
    pub candidate: Candidate,
    pub authentication: Result<AuthenticatedHost, ProbeFailure>,
}

#[derive(Debug)]
pub struct ProbedScan {
    pub candidates: Vec<ProbedCandidate>,
    pub issues: Vec<ScanIssue>,
    pub limit: Option<ScanLimit>,
}

impl Scan {
    /// At most one probe is outstanding, with a fixed shared two-second budget.
    /// Unprobed and failed candidates remain explicit lifecycle observations.
    pub async fn probe(self) -> ProbedScan {
        self.probe_until(Instant::now() + TOTAL_PROBE_BUDGET).await
    }

    async fn probe_until(self, deadline: Instant) -> ProbedScan {
        let mut candidates = Vec::with_capacity(self.candidates.len());
        for candidate in self.candidates {
            let now = Instant::now();
            let authentication = if now >= deadline {
                Err(ProbeFailure::BudgetExhausted)
            } else {
                candidate
                    .authenticate(deadline.min(now + PROBE_BUDGET))
                    .await
                    .map_err(ProbeFailure::Indeterminate)
            };
            candidates.push(ProbedCandidate {
                candidate,
                authentication,
            });
        }
        ProbedScan {
            candidates,
            issues: self.issues,
            limit: self.limit,
        }
    }
}

#[derive(Debug)]
struct ReadBudget {
    remaining: usize,
}

#[derive(Debug)]
struct ReadBudgetExhausted;
impl std::fmt::Display for ReadBudgetExhausted {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("registry metadata read budget exhausted")
    }
}
impl std::error::Error for ReadBudgetExhausted {}

impl ReadBudget {
    fn read(&mut self, directory: &Directory, name: &str) -> io::Result<Option<(File, Vec<u8>)>> {
        let mut file = match directory.open_read(OsStr::new(name)) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            value => value?,
        };
        let mut bytes = Vec::new();
        let mut chunk = [0_u8; 8192];
        loop {
            if bytes.len() > MAX_METADATA_BYTES {
                return Err(invalid("endpoint metadata exceeds size limit"));
            }
            if self.remaining == 0 {
                return Err(io::Error::other(ReadBudgetExhausted));
            }
            let count = chunk
                .len()
                .min(self.remaining)
                .min(MAX_METADATA_BYTES + 1 - bytes.len());
            let read = file.read(&mut chunk[..count])?;
            self.remaining -= read;
            if read == 0 {
                return Ok(Some((file, bytes)));
            }
            bytes.extend_from_slice(&chunk[..read]);
        }
    }

    fn verify(&mut self, issued: &Issued) -> io::Result<()> {
        let parent = issued
            .path
            .parent()
            .ok_or_else(|| invalid("publication path has no parent"))?;
        let directory = Directory::open_existing(parent, true)?;
        let (file, bytes) = self.read(&directory, &issued.name)?.ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                "observed registry path disappeared",
            )
        })?;
        if locking::file_key(&file)? != locking::file_key(&issued.file)? || bytes != issued.bytes {
            return Err(invalid("registry path changed identity or contents"));
        }
        Ok(())
    }
}

impl RegistrySet {
    pub fn scan_namespaces(&self) -> Scan {
        self.scan(false, MAX_SCAN_ENTRIES, MAX_SCAN_ROWS, MAX_SCAN_BYTES)
    }
    pub fn scan_inventory(&self) -> Scan {
        self.scan(true, MAX_SCAN_ENTRIES, MAX_SCAN_ROWS, MAX_SCAN_BYTES)
    }

    /// Exact configured row keys, including this namespace's inventory key.
    /// Unrelated inventory rows and broad scan/probe budgets cannot hide these
    /// recovery observations. Missing rows are normal; bad rows remain errors.
    pub fn observe(&self, id: &str) -> io::Result<Scan> {
        self.observe_filtered(id, true)
    }

    pub(super) fn observe_filtered(&self, id: &str, inventory: bool) -> io::Result<Scan> {
        metadata::validate_id(id)?;
        let mut scan = Scan::default();
        let mut budget = ReadBudget {
            remaining: MAX_SCAN_BYTES,
        };
        for (index, root) in self
            .0
            .iter()
            .enumerate()
            .filter(|(_, root)| inventory || !root.inventory)
        {
            let name = format!("{}.json", self.record_key(root, id));
            match self.observe_row(index, &name, &mut budget) {
                Ok(Some(candidate)) => scan.candidates.push(candidate),
                Ok(None) => {}
                Err(error) => scan.issues.push(ScanIssue {
                    root: root.path.clone(),
                    name: Some(name),
                    error,
                }),
            }
        }
        Ok(scan)
    }

    fn observe_row(
        &self,
        index: usize,
        name: &str,
        budget: &mut ReadBudget,
    ) -> io::Result<Option<Candidate>> {
        let root = &self.0[index];
        let Some((file, bytes)) = budget.read(&root.directory, name)? else {
            return Ok(None);
        };
        let record = RegistryRecord::from_json(&bytes)?;
        if name.get(..crate::workspace::WORKSPACE_ID_LENGTH) != Some(record.host.id.as_str()) {
            return Err(invalid("registry filename workspace identity mismatch"));
        }
        let issued = Issued {
            directory: root.directory.clone(),
            path: root.path.join(name),
            name: name.to_owned(),
            file,
            bytes,
            registry: true,
        };
        budget.verify(&issued)?;
        Ok(Some(Candidate {
            issued,
            source: Source::Row {
                registries: self.clone(),
                root: index,
                inventory: root.inventory,
            },
            metadata: record.host.clone(),
            record: Some(record),
        }))
    }

    // Private smaller bounds permit deterministic boundary fixtures without
    // constructing thousands of files. Public callers use the fixed limits.
    fn scan(
        &self,
        inventory: bool,
        entries_limit: usize,
        rows_limit: usize,
        bytes_limit: usize,
    ) -> Scan {
        let mut scan = Scan::default();
        let mut entries_left = entries_limit;
        let mut rows = 0;
        let mut budget = ReadBudget {
            remaining: bytes_limit,
        };
        for (index, root) in self
            .0
            .iter()
            .enumerate()
            .filter(|(_, root)| root.inventory == inventory)
        {
            let (names, truncated) = match root.directory.entries(entries_left) {
                Ok(value) => value,
                Err(error) => {
                    scan.issues.push(ScanIssue {
                        root: root.path.clone(),
                        name: None,
                        error,
                    });
                    continue;
                }
            };
            entries_left -= names.len();
            for name in names {
                let Some(name) = name.to_str().filter(|name| registry_name(name, inventory)) else {
                    continue;
                };
                if rows == rows_limit {
                    scan.limit = Some(ScanLimit::Rows);
                    return scan;
                }
                rows += 1;
                match self.observe_row(index, name, &mut budget) {
                    Ok(Some(candidate)) => scan.candidates.push(candidate),
                    Ok(None) => scan.issues.push(ScanIssue {
                        root: root.path.clone(),
                        name: Some(name.to_owned()),
                        error: io::Error::new(
                            io::ErrorKind::NotFound,
                            "registry row disappeared during scan",
                        ),
                    }),
                    Err(error)
                        if error
                            .get_ref()
                            .is_some_and(|cause| cause.is::<ReadBudgetExhausted>()) =>
                    {
                        scan.limit = Some(ScanLimit::MetadataBytes);
                        return scan;
                    }
                    Err(error) => scan.issues.push(ScanIssue {
                        root: root.path.clone(),
                        name: Some(name.to_owned()),
                        error,
                    }),
                }
            }
            if truncated {
                scan.limit = Some(ScanLimit::Entries);
                return scan;
            }
        }
        scan
    }
}
fn registry_name(name: &str, inventory: bool) -> bool {
    let Some(stem) = name.strip_suffix(".json") else {
        return false;
    };
    let id_length = crate::workspace::WORKSPACE_ID_LENGTH;
    let expected = if inventory {
        id_length + 1 + 32
    } else {
        id_length
    };
    stem.len() == expected
        && stem.bytes().enumerate().all(|(index, byte)| {
            if inventory && index == id_length {
                byte == b'-'
            } else {
                byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
            }
        })
}

impl EndpointLocation {
    /// Unlike an inventory hint, this independently configured location already
    /// owns the namespace roots needed to lock and retire its ready record.
    pub fn observe_ready(&self) -> io::Result<Option<Candidate>> {
        let directory = match Directory::open_existing(&self.directory, true) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            result => Arc::new(result?),
        };
        let Some((file, bytes)) = read_optional(&directory, READY_NAME)? else {
            return Ok(None);
        };
        let metadata = EndpointMetadata::from_json(&bytes)?;
        if metadata.project_root_bytes != encode_path(&self.project) {
            return Err(invalid("ready record workspace identity mismatch"));
        }
        let issued = Issued {
            directory,
            path: self.ready_record(),
            name: READY_NAME.to_owned(),
            file,
            bytes,
            registry: false,
        };
        issued.verify_path()?;
        Ok(Some(Candidate {
            issued,
            source: Source::Ready(self.clone()),
            metadata,
            record: None,
        }))
    }
}

#[cfg(test)]
mod tests;
