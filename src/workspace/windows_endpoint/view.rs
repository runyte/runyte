// SPDX-License-Identifier: MPL-2.0

use super::*;

/// Existing configured roots admitted without creating files or changing ACLs.
/// An inventory-only view has no implicit namespace or publication authority.
#[derive(Clone, Debug)]
pub struct RegistryView {
    registries: RegistrySet,
    namespaces_complete: bool,
}

impl RegistryView {
    pub fn open(paths: &[PathBuf], inventory: Option<PathBuf>) -> io::Result<Self> {
        if paths.len() + usize::from(inventory.is_some()) > MAX_REGISTRIES {
            return Err(invalid("at most three registry roots may be observed"));
        }
        let mut complete = !paths.is_empty();
        let mut roots = Vec::new();
        for (path, inventory) in paths
            .iter()
            .map(|path| (path, false))
            .chain(inventory.as_ref().map(|path| (path, true)))
        {
            metadata::persisted_path(&encode_path(path))?;
            let directory = match Directory::open_existing(path, true) {
                Ok(directory) => Arc::new(directory),
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    if !inventory {
                        complete = false;
                    }
                    continue;
                }
                Err(error) => return Err(error),
            };
            // Even an empty existing root without its stable identity is
            // indeterminate. A reader must not manufacture that identity.
            let lock = directory.open_read(OsStr::new(REGISTRY_LOCK))?;
            roots.push(RegistryRoot {
                directory,
                path: path.clone(),
                key: locking::file_key(&lock)?,
                inventory,
            });
        }
        Ok(Self {
            registries: RegistrySet::from_roots(roots)?,
            namespaces_complete: complete,
        })
    }

    pub fn scan_namespaces(&self) -> Scan {
        self.registries.scan_namespaces()
    }

    pub fn scan_inventory(&self) -> Scan {
        self.registries.scan_inventory()
    }

    /// Exact namespace rows, independent of broad enumeration limits.
    pub fn observe_namespaces(&self, id: &str) -> io::Result<Scan> {
        self.registries.observe_filtered(id, false)
    }

    /// Includes the configured namespace's inventory key only when every
    /// namespace identity was admitted. Never hashes a partial namespace set.
    pub fn observe(&self, id: &str) -> io::Result<Scan> {
        if !self.namespaces_complete && self.registries.0.iter().any(|root| root.inventory) {
            return Err(invalid(
                "exact inventory observation requires every namespace identity",
            ));
        }
        self.registries.observe(id)
    }

    /// Observes exact stored native identity without recanonicalizing a
    /// possibly missing project. This candidate has no ready-cleanup authority.
    pub(crate) fn observe_snapshot_ready(
        &self,
        known: &crate::workspace::windows_location::KnownReadLocation,
    ) -> io::Result<Option<Candidate>> {
        discovery::observe_snapshot_ready(known.project_root(), known.endpoint_directory())
    }

    /// Read the configured ready record even when registry roots are absent.
    /// Such a candidate still needs native peer authentication; without the
    /// namespace identities, exact ready-record retirement is unavailable.
    pub fn observe_ready(
        &self,
        project: &Path,
        directory: PathBuf,
    ) -> io::Result<Option<Candidate>> {
        let location = EndpointLocation::new(project, directory, self.registries.clone())?;
        let mut candidate = location.observe_ready()?;
        if !self.namespaces_complete
            && let Some(candidate) = &mut candidate
        {
            candidate.disallow_unscoped_ready_cleanup();
        }
        Ok(candidate)
    }
}

#[cfg(test)]
#[path = "view/tests.rs"]
mod tests;
