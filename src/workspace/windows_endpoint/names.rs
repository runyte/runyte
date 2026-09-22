// SPDX-License-Identifier: MPL-2.0

//! Persisted names and publication-owned live updates. Replacement deliberately
//! has an unlink/install gap to preserve a racing foreign file. Known locks
//! serialize cooperating writers; this is not a crash-atomic multi-file journal.

use super::*;
use std::{ffi::OsString, io::Write};

const STORE_LOCK: &str = ".host-names.lock";
const MAX_STORED_NAME_BYTES: usize = 1024;

fn validate_name(name: &str) -> io::Result<()> {
    crate::workspace::session_name::validate_host_name(name)
        .map_err(|error| invalid(&error.to_string()))
}

/// Configured state storage, independent of endpoint runtime placement and of
/// untrusted metadata. Names are persisted, not guaranteed power-loss durable.
#[derive(Clone, Debug)]
pub struct NameStore {
    directory: Arc<Directory>,
    path: PathBuf,
}

impl NameStore {
    pub fn open(state_root: &Path) -> io::Result<Self> {
        let path = state_root.join("host-names");
        metadata::persisted_path(&encode_path(&path))?;
        Ok(Self {
            directory: Arc::new(Directory::open(&path, true)?),
            path,
        })
    }

    fn lock(&self) -> io::Result<locking::Lock> {
        let file = self.directory.append(OsStr::new(STORE_LOCK))?;
        let key = locking::file_key(&file)?;
        let lock = locking::Lock::acquire(file)?;
        let current =
            Directory::open_existing(&self.path, true)?.open_read(OsStr::new(STORE_LOCK))?;
        if locking::file_key(&current)? != key {
            return Err(invalid("stored-name lock or directory changed identity"));
        }
        Ok(lock)
    }

    fn read_unlocked(&self, id: &str) -> io::Result<Option<(File, Vec<u8>, String)>> {
        metadata::validate_id(id)?;
        let name = format!("{id}.json");
        let Some((file, bytes)) = read_bounded(&self.directory, &name, MAX_STORED_NAME_BYTES)?
        else {
            return Ok(None);
        };
        let value: String = serde_json::from_slice(&bytes)
            .map_err(|error| invalid(&format!("malformed stored session name: {error}")))?;
        validate_name(&value)?;
        verify_at(&self.path.join(&name), &file, &bytes, MAX_STORED_NAME_BYTES)?;
        Ok(Some((file, bytes, value)))
    }

    /// Takes the stable store lock: a live rename gap must never look like an
    /// absent explicit name to a concurrent automatic allocator.
    pub fn load(&self, id: &str) -> io::Result<Option<String>> {
        metadata::validate_id(id)?;
        let _store = self.lock()?;
        Ok(self.read_unlocked(id)?.map(|(_, _, name)| name))
    }

    /// Keeps an existing explicit name; otherwise reserves an available name
    /// under the same registry locks used by initial and live publications.
    pub fn store_if_absent(
        &self,
        registries: &RegistrySet,
        id: &str,
        name: &str,
    ) -> io::Result<String> {
        self.store_if_absent_with(registries, id, name, |_| Ok(()))
    }

    fn store_if_absent_with(
        &self,
        registries: &RegistrySet,
        id: &str,
        name: &str,
        after_write: impl FnOnce(&Path) -> io::Result<()>,
    ) -> io::Result<String> {
        metadata::validate_id(id)?;
        validate_name(name)?;
        let _identity = registries.identity_locks(id)?;
        let _registry = registries.registry_locks()?;
        let _store = self.lock()?;
        if let Some((_, _, existing)) = self.read_unlocked(id)? {
            return Ok(existing);
        }
        ensure_available(registries, id, name)?;
        let leaf = format!("{id}.json");
        let bytes = serde_json::to_vec(name).map_err(io::Error::other)?;
        let file = self
            .directory
            .atomic_write_owned(OsStr::new(&leaf), &bytes)?;
        let path = self.path.join(&leaf);
        if let Err(primary) =
            after_write(&path).and_then(|()| verify_at(&path, &file, &bytes, MAX_STORED_NAME_BYTES))
        {
            let cleanup = self
                .directory
                .remove_owned(OsStr::new(&leaf), &file)
                .and_then(|()| self.directory.sync());
            return Err(match cleanup {
                Ok(()) => primary,
                Err(error) => io::Error::new(
                    primary.kind(),
                    format!("{primary}; stored-name rollback left unverified residue: {error}"),
                ),
            });
        }
        Ok(name.to_owned())
    }
}

pub(super) fn ensure_available(
    registries: &RegistrySet,
    owner: &str,
    name: &str,
) -> io::Result<()> {
    validate_name(name)?;
    let scan = registries.scan_namespaces();
    ensure_scan_available(&scan, owner, name)
}

fn ensure_scan_available(scan: &Scan, owner: &str, name: &str) -> io::Result<()> {
    if scan.limit.is_some() || !scan.issues.is_empty() {
        return Err(invalid(
            "session name availability is indeterminate: namespace scan incomplete",
        ));
    }
    if scan.candidates.iter().any(|candidate| {
        candidate.metadata().id != owner && candidate.metadata().name.as_deref() == Some(name)
    }) {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "session name is already reserved in this namespace",
        ));
    }
    Ok(())
}

impl EndpointLocation {
    pub fn prepare_named(
        &self,
        names: &NameStore,
        requested: Option<String>,
    ) -> io::Result<PreparedEndpoint> {
        // prepare owns identity and registry guards; never recursively acquire
        // them while reading the name store.
        let mut prepared = self.prepare(requested)?;
        let _store = names.lock()?;
        if prepared.metadata.name.is_none() {
            prepared.metadata.name = names
                .read_unlocked(&prepared.metadata.id)?
                .map(|(_, _, name)| name);
            if let Some(name) = &prepared.metadata.name {
                ensure_available(&self.registries, &prepared.metadata.id, name)?;
            }
        }
        Ok(prepared)
    }
}

#[derive(Debug)]
struct Snapshot {
    file: File,
    bytes: Vec<u8>,
}

#[derive(Debug)]
struct Pending {
    file: File,
    name: OsString,
}

#[derive(Debug)]
struct Change {
    directory: Arc<Directory>,
    path: PathBuf,
    name: String,
    old: Option<Snapshot>,
    bytes: Vec<u8>,
    next: Option<Pending>,
    restore: Option<Pending>,
    touched: bool,
    // None is the persistent name; metadata indices point into Publication.
    publication_index: Option<usize>,
}

/// At most one transaction, with <= MAX_REGISTRIES + 2 records and at most
/// one new and one restoration staging file per record. Retrying reuses those
/// handles and never accumulates generations after a failed rollback.
#[derive(Debug)]
pub(super) struct RenameRecovery {
    store: NameStore,
    changes: Vec<Change>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Step {
    Staged,
    Removing,
    Removed,
    Installed,
    Synced,
    RollingBack,
    Restored,
}

impl Publication {
    // Retained-ledger fixture for the publication-worker ownership boundary.
    // Inject after a successful install, then block rollback; not an OS fault.
    #[cfg(test)]
    pub(crate) fn fixture_pending_name_update(
        &mut self,
        names: &NameStore,
        name: &str,
    ) -> io::Result<()> {
        self.rename_with(names, name, |index, step, _| {
            if (index == 1 && step == Step::Installed) || step == Step::RollingBack {
                Err(io::Error::other("injected retained name-update recovery"))
            } else {
                Ok(())
            }
        })
    }

    pub fn rename_recovery_pending(&self) -> bool {
        self.rename_recovery.is_some()
    }

    /// Synchronous bounded filesystem work; run off the editor loop. Never
    /// await pipe operations while holding these identity/registry/store locks.
    pub fn rename(&mut self, names: &NameStore, name: &str) -> io::Result<()> {
        self.rename_with(names, name, |_, _, _| Ok(()))
    }

    fn rename_with(
        &mut self,
        names: &NameStore,
        name: &str,
        mut hook: impl FnMut(usize, Step, &Path) -> io::Result<()>,
    ) -> io::Result<()> {
        validate_name(name)?;
        if self.retiring || self.rename_recovery.is_some() {
            return Err(io::Error::other(
                "publication is retiring or has a pending name-update recovery",
            ));
        }
        let _identity = self.location.registries.identity_locks(&self.metadata.id)?;
        let _registry = self.location.registries.registry_locks()?;
        let _store = names.lock()?;
        ensure_available(&self.location.registries, &self.metadata.id, name)?;
        for issued in &self.issued {
            issued.verify_path()?;
        }
        let mut metadata = self.metadata.clone();
        metadata.name = Some(name.to_owned());
        let registry_bytes = RegistryRecord {
            host: metadata.clone(),
            ready_record_bytes: encode_path(&self.location.ready_record()),
        }
        .to_json()?;
        let ready_bytes = metadata.to_json()?;
        let stored = names.read_unlocked(&self.metadata.id)?;
        let store_name = format!("{}.json", self.metadata.id);
        let mut changes = vec![Change {
            directory: names.directory.clone(),
            path: names.path.join(&store_name),
            name: store_name,
            old: stored.map(|(file, bytes, _)| Snapshot { file, bytes }),
            bytes: serde_json::to_vec(name).map_err(io::Error::other)?,
            next: None,
            restore: None,
            touched: false,
            publication_index: None,
        }];
        for (index, issued) in self.issued.iter().enumerate() {
            changes.push(Change {
                directory: issued.directory.clone(),
                path: issued.path.clone(),
                name: issued.name.clone(),
                old: Some(Snapshot {
                    file: issued.file.try_clone()?,
                    bytes: issued.bytes.clone(),
                }),
                bytes: if issued.registry {
                    registry_bytes.clone()
                } else {
                    ready_bytes.clone()
                },
                next: None,
                restore: None,
                touched: false,
                publication_index: Some(index),
            });
        }
        // Ownership is recorded before any staging/write/install operation.
        self.rename_recovery = Some(RenameRecovery {
            store: names.clone(),
            changes,
        });
        let result = self.apply_rename(&mut hook);
        if let Err(primary) = result {
            return match self.rollback_rename(&mut hook) {
                Ok(()) => Err(primary),
                Err(rollback) => Err(io::Error::new(
                    primary.kind(),
                    format!("{primary}; name-update rollback requires recovery: {rollback}"),
                )),
            };
        }
        self.metadata = metadata;
        self.rename_recovery = None;
        Ok(())
    }

    fn apply_rename(
        &mut self,
        hook: &mut impl FnMut(usize, Step, &Path) -> io::Result<()>,
    ) -> io::Result<()> {
        let recovery = self
            .rename_recovery
            .as_mut()
            .expect("rename owns recovery ledger");
        for (index, change) in recovery.changes.iter_mut().enumerate() {
            let (name, file) = change.directory.create_owned_pending()?;
            change.next = Some(Pending { file, name });
            let pending = change.next.as_mut().unwrap();
            pending.file.write_all(&change.bytes)?;
            pending.file.sync_all()?;
            hook(index, Step::Staged, &change.path)?;
        }
        for (index, change) in recovery.changes.iter_mut().enumerate() {
            change.verify_old()?;
            hook(index, Step::Removing, &change.path)?;
            change.touched = true;
            if let Some(old) = &change.old {
                change
                    .directory
                    .remove_owned(OsStr::new(&change.name), &old.file)?;
            }
            hook(index, Step::Removed, &change.path)?;
            let pending = change.next.as_ref().unwrap();
            change
                .directory
                .install_owned_pending(&pending.file, OsStr::new(&change.name))?;
            hook(index, Step::Installed, &change.path)?;
            change.directory.sync()?;
            hook(index, Step::Synced, &change.path)?;
        }
        for change in &recovery.changes {
            verify_at(
                &change.path,
                &change.next.as_ref().unwrap().file,
                &change.bytes,
                change.limit(),
            )?;
        }
        // Clone every new publication handle before changing the old ownership
        // vector. A clone failure still leaves a complete rollback ledger.
        let replacements = recovery
            .changes
            .iter()
            .filter_map(|change| change.publication_index.map(|index| (index, change)))
            .map(|(index, change)| {
                Ok((
                    index,
                    change.next.as_ref().unwrap().file.try_clone()?,
                    change.bytes.clone(),
                ))
            })
            .collect::<io::Result<Vec<_>>>()?;
        for (index, file, bytes) in replacements {
            self.issued[index].file = file;
            self.issued[index].bytes = bytes;
        }
        Ok(())
    }

    pub fn retry_rename_recovery(&mut self) -> io::Result<()> {
        if self.retiring {
            return Err(io::Error::other("publication retirement must use cleanup"));
        }
        let Some(store) = self
            .rename_recovery
            .as_ref()
            .map(|value| value.store.clone())
        else {
            return Ok(());
        };
        let _identity = self.location.registries.identity_locks(&self.metadata.id)?;
        let _registry = self.location.registries.registry_locks()?;
        let _store = store.lock()?;
        self.rollback_rename(&mut |_, _, _| Ok(()))
    }

    fn rollback_rename(
        &mut self,
        hook: &mut impl FnMut(usize, Step, &Path) -> io::Result<()>,
    ) -> io::Result<()> {
        let recovery = self
            .rename_recovery
            .as_mut()
            .expect("rollback owns recovery ledger");
        let mut failure = None;
        for (index, change) in recovery.changes.iter_mut().enumerate().rev() {
            if let Err(error) = hook(index, Step::RollingBack, &change.path)
                .and_then(|()| change.rollback())
                .and_then(|()| hook(index, Step::Restored, &change.path))
            {
                failure.get_or_insert(error);
            }
        }
        if let Some(error) = failure {
            return Err(error);
        }
        let restored = recovery
            .changes
            .iter()
            .filter_map(|change| change.publication_index.map(|index| (index, change)))
            .map(|(index, change)| {
                let old = change
                    .old
                    .as_ref()
                    .expect("publication always had previous bytes");
                let file = change
                    .restore
                    .as_ref()
                    .map_or(&old.file, |value| &value.file)
                    .try_clone()?;
                Ok((index, file, old.bytes.clone()))
            })
            .collect::<io::Result<Vec<_>>>()?;
        for (index, file, bytes) in restored {
            self.issued[index].file = file;
            self.issued[index].bytes = bytes;
        }
        self.rename_recovery = None;
        Ok(())
    }

    pub(super) fn cleanup_rename_recovery(&mut self) -> io::Result<()> {
        let Some(recovery) = self.rename_recovery.as_mut() else {
            return Ok(());
        };
        let store = recovery.store.lock();
        let mut failure = store
            .as_ref()
            .err()
            .map(|error| io::Error::new(error.kind(), error.to_string()));
        for change in recovery.changes.iter_mut().rev() {
            let result = if change.publication_index.is_none() {
                if store.is_ok() {
                    change.rollback()
                } else {
                    continue;
                }
            } else {
                change.cleanup_all()
            };
            if let Err(error) = result {
                failure.get_or_insert(error);
            }
        }
        if let Some(error) = failure {
            return Err(error);
        }
        self.rename_recovery = None;
        Ok(())
    }
}

impl Change {
    fn limit(&self) -> usize {
        if self.publication_index.is_some() {
            MAX_METADATA_BYTES
        } else {
            MAX_STORED_NAME_BYTES
        }
    }

    fn verify_old(&self) -> io::Result<()> {
        match &self.old {
            Some(old) => verify_at(&self.path, &old.file, &old.bytes, self.limit()),
            None => require_absent(&self.directory, &self.name),
        }
    }

    fn cleanup_pending(&self, pending: &Pending, include_final: bool) -> io::Result<()> {
        let mut failure = None;
        for name in std::iter::once(pending.name.as_os_str())
            .chain(include_final.then_some(OsStr::new(&self.name)))
        {
            if let Err(error) = self.directory.remove_owned(name, &pending.file)
                && error.kind() != io::ErrorKind::NotFound
            {
                failure.get_or_insert(error);
            }
        }
        if let Some(error) = failure {
            return Err(error);
        }
        Ok(())
    }

    fn rollback(&mut self) -> io::Result<()> {
        if !self.touched {
            if let Some(next) = &self.next {
                self.cleanup_pending(next, false)?;
            }
            return Ok(());
        }
        let current = read_bounded(&self.directory, &self.name, self.limit())?;
        let old_is_current = match (&current, &self.old) {
            (Some((file, bytes)), Some(old)) => {
                locking::file_key(file)? == locking::file_key(&old.file)? && bytes == &old.bytes
            }
            (None, None) => true,
            _ => false,
        };
        let restored_is_current = match (&current, &self.restore, &self.old) {
            (Some((file, bytes)), Some(restore), Some(old)) => {
                locking::file_key(file)? == locking::file_key(&restore.file)? && bytes == &old.bytes
            }
            _ => false,
        };
        if !old_is_current && !restored_is_current {
            if let Some((file, bytes)) = current {
                let next = self.next.as_ref().expect("touched change was staged");
                if locking::file_key(&file)? != locking::file_key(&next.file)?
                    || bytes != self.bytes
                {
                    return Err(invalid(
                        "name rollback preserves a foreign or changed record",
                    ));
                }
                self.directory
                    .remove_owned(OsStr::new(&self.name), &next.file)?;
            }
            if let Some(old) = &self.old {
                if self.restore.is_none() {
                    let (name, file) = self.directory.create_owned_pending()?;
                    self.restore = Some(Pending { file, name });
                }
                let restore = self.restore.as_mut().unwrap();
                // Retry a partial staging write in place; the single retained
                // restoration file keeps failed-recovery storage bounded.
                use std::io::{Seek, SeekFrom};
                restore.file.set_len(0)?;
                restore.file.seek(SeekFrom::Start(0))?;
                restore.file.write_all(&old.bytes)?;
                restore.file.sync_all()?;
                self.directory
                    .install_owned_pending(&restore.file, OsStr::new(&self.name))?;
            }
        }
        self.directory.sync()?;
        if let Some(old) = &self.old {
            let file = self
                .restore
                .as_ref()
                .map_or(&old.file, |restore| &restore.file);
            verify_at(&self.path, file, &old.bytes, self.limit())?;
        } else {
            require_absent(&self.directory, &self.name)?;
        }
        if let Some(next) = &self.next {
            self.cleanup_pending(next, true)?;
        }
        Ok(())
    }

    fn cleanup_all(&self) -> io::Result<()> {
        let mut failure = None;
        for pending in [&self.next, &self.restore].into_iter().flatten() {
            if let Err(error) = self.cleanup_pending(pending, true) {
                failure.get_or_insert(error);
            }
        }
        if let Some(old) = &self.old
            && let Err(error) = self
                .directory
                .remove_owned(OsStr::new(&self.name), &old.file)
            && error.kind() != io::ErrorKind::NotFound
        {
            failure.get_or_insert(error);
        }
        if let Err(error) = self.directory.sync() {
            failure.get_or_insert(error);
        }
        if let Some(error) = failure {
            return Err(error);
        }
        Ok(())
    }
}

fn read_bounded(
    directory: &Directory,
    name: &str,
    limit: usize,
) -> io::Result<Option<(File, Vec<u8>)>> {
    let mut file = match directory.open_read(OsStr::new(name)) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        value => value?,
    };
    let mut bytes = Vec::new();
    (&mut file)
        .take((limit + 1) as u64)
        .read_to_end(&mut bytes)?;
    if bytes.len() > limit {
        return Err(invalid("session-name record exceeds size limit"));
    }
    Ok(Some((file, bytes)))
}

fn verify_at(path: &Path, expected: &File, bytes: &[u8], limit: usize) -> io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| invalid("name record has no parent"))?;
    let name = path
        .file_name()
        .and_then(OsStr::to_str)
        .ok_or_else(|| invalid("invalid name record leaf"))?;
    let directory = Directory::open_existing(parent, true)?;
    let (file, current) = read_bounded(&directory, name, limit)?
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "name record path disappeared"))?;
    if locking::file_key(&file)? != locking::file_key(expected)? || current != bytes {
        return Err(invalid("name record path changed identity or bytes"));
    }
    Ok(())
}

#[cfg(test)]
mod tests;
