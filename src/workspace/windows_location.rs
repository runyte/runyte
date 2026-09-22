// SPDX-License-Identifier: MPL-2.0

//! Captured configured native locations. Resolution observes existing storage;
//! publication is an explicit, separate operation. Nothing here enables the
//! persistent-session CLI or derives a namespace from an inventory row.

use super::{
    WorkspaceIdentity,
    windows_endpoint::{
        self, EndpointLocation, MAX_PERSISTED_PATH_BYTES, RegistrySet, RegistryView,
    },
};
use crate::{native_path::encode_path, private_storage::Directory};
use std::{
    ffi::{OsStr, OsString},
    io,
    os::windows::ffi::OsStrExt,
    path::{Component, Path, PathBuf, Prefix},
};

/// Internal launch consistency marker, not a source of authority or paths.
pub const EXPECTED_LAYOUT_ENV: &str = "RUNYTE_INTERNAL_WINDOWS_LAYOUT";

/// A single snapshot of inherited settings and the OS account default. Tests
/// supply these values directly; they never need the account's real folders.
#[derive(Clone, Debug, Default)]
pub struct CapturedRoots {
    pub runtime_root: Option<PathBuf>,
    pub cache_home: Option<PathBuf>,
    pub local_app_data: Option<PathBuf>,
    pub inventory_override: Option<PathBuf>,
}

impl CapturedRoots {
    pub fn capture() -> Self {
        Self {
            runtime_root: std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from),
            cache_home: std::env::var_os("XDG_CACHE_HOME")
                .map(PathBuf::from)
                .filter(|path| path.is_absolute()),
            local_app_data: crate::user_paths::system_local_app_data_directory(),
            inventory_override: std::env::var_os("RUNYTE_ALL_HOSTS_DIR").map(PathBuf::from),
        }
    }

    fn cache_root(&self) -> Option<PathBuf> {
        self.cache_home
            .as_ref()
            .filter(|path| path.is_absolute())
            .map(|path| path.join("runyte"))
            .or_else(|| {
                self.local_app_data
                    .as_ref()
                    .map(|path| path.join("runyte").join("cache"))
            })
    }

    /// Deliberately lazy: normal discovery does not depend on this root.
    pub fn inventory_root(&self) -> io::Result<PathBuf> {
        let path = self
            .inventory_override
            .clone()
            .or_else(|| {
                self.local_app_data
                    .as_deref()
                    .map(windows_endpoint::inventory_root_in)
            })
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::NotFound,
                    "account inventory root unavailable",
                )
            })?;
        validate_path(&path)?;
        Ok(path)
    }
}

/// Project discovery/configuration supplies the state root and all independently
/// reserved application-storage paths. The selected cache is also reserved;
/// inventory separation is checked only when preparing publication.
pub struct LocationInputs {
    pub project_root: PathBuf,
    pub state_root: PathBuf,
    pub reserved_user_roots: Vec<PathBuf>,
    pub roots: CapturedRoots,
}

#[derive(Clone, Debug)]
pub struct ResolvedLayout {
    project: PathBuf,
    state: PathBuf,
    endpoint: PathBuf,
    namespaces: Vec<PathBuf>,
    names: PathBuf,
    runtime: Option<PathBuf>,
    cache: Option<PathBuf>,
    roots: CapturedRoots,
    cache_error: Option<String>,
    reserved: Vec<PathBuf>,
}

/// A configured observation address, never a publication constructor.
/// Its project may have disappeared after its canonical identity was recorded.
#[derive(Clone, Debug)]
pub struct KnownReadLocation {
    project: PathBuf,
    directory: PathBuf,
    namespaces: Vec<PathBuf>,
}

impl KnownReadLocation {
    pub fn project_root(&self) -> &Path {
        &self.project
    }
    pub fn endpoint_directory(&self) -> &Path {
        &self.directory
    }
    pub(crate) fn namespace_roots(&self) -> &[PathBuf] {
        &self.namespaces
    }
}

impl ResolvedLayout {
    /// Captures fallback choices without creating or hardening any directory.
    /// A missing cache can be prepared later beneath an admitted local NTFS
    /// ancestor. Read admission does not promise future write access; a later
    /// publication failure never silently changes this frozen layout.
    pub fn resolve(inputs: LocationInputs) -> io::Result<Self> {
        validate_path(&inputs.state_root)?;
        let project = WorkspaceIdentity::resolve(&inputs.project_root)?
            .root()
            .to_owned();
        validate_path(&project)?;
        let cache_candidate = inputs.roots.cache_root();
        let runtime = inputs
            .roots
            .runtime_root
            .as_ref()
            .filter(|path| {
                validate_path(path).is_ok() && Directory::open_existing(path, true).is_ok()
            })
            .cloned();
        let (cache, cache_error) = match cache_candidate {
            Some(path) => match admit_cache(&path) {
                Ok(()) => (Some(path), None),
                Err(error) => (
                    None,
                    Some(format!("configured cache is unavailable: {error}")),
                ),
            },
            None => (None, None),
        };
        let mut reserved = inputs.reserved_user_roots;
        if let Some(cache) = &cache {
            reserved.push(cache.clone());
        }
        validate_state_separation(&inputs.state_root, &reserved)?;
        let endpoint = runtime.as_ref().map_or_else(
            || inputs.state_root.join("host"),
            |root| root.join("runyte").join(super::workspace_id(&project)),
        );
        let mut namespaces = Vec::new();
        if let Some(cache) = &cache {
            namespaces.push(cache.join("hosts"));
        }
        if let Some(runtime) = &runtime {
            namespaces.push(runtime.join("runyte").join("hosts"));
        }
        let names = inputs.state_root.join("host-names");
        validate_path(&endpoint.join("endpoint.json"))?;
        validate_path(&names)?;
        for root in &namespaces {
            validate_path(root)?;
        }
        Ok(Self {
            project,
            state: inputs.state_root,
            endpoint,
            namespaces,
            names,
            runtime,
            cache,
            roots: inputs.roots,
            cache_error,
            reserved,
        })
    }

    pub fn project_root(&self) -> &Path {
        &self.project
    }
    pub fn state_root(&self) -> &Path {
        &self.state
    }
    pub fn endpoint_directory(&self) -> &Path {
        &self.endpoint
    }
    pub fn namespace_roots(&self) -> &[PathBuf] {
        &self.namespaces
    }
    pub fn name_store_root(&self) -> &Path {
        &self.names
    }

    /// The selected optional history cache. Does not prepare or harden it.
    pub fn cache_root(&self) -> io::Result<Option<&Path>> {
        if let Some(error) = &self.cache_error {
            return Err(io::Error::other(error.clone()));
        }
        Ok(self.cache.as_deref())
    }

    /// A read-only snapshot of this already resolved project's ready location.
    pub(crate) fn read_location(&self) -> KnownReadLocation {
        KnownReadLocation {
            project: self.project.clone(),
            directory: self.endpoint.clone(),
            namespaces: self.namespaces.clone(),
        }
    }

    /// Derives only an observation address from a stored canonical identity.
    /// Missing projects retain their bytes. Existing identity changes, denied
    /// resolution and unsafe paths fail rather than silently retarget history.
    /// The frozen runtime/cache selection is never readmitted or recaptured.
    pub fn known_read_location(
        &self,
        project: &Path,
        state: &Path,
    ) -> io::Result<KnownReadLocation> {
        validate_path(project)?;
        validate_path(state)?;
        match project.canonicalize() {
            Ok(current) if encode_path(&current) != encode_path(project) => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "remembered project directory identity changed",
                ));
            }
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        validate_state_separation(state, &self.reserved)?;
        let directory = self.runtime.as_ref().map_or_else(
            || state.join("host"),
            |runtime| runtime.join("runyte").join(super::workspace_id(project)),
        );
        validate_path(&directory.join("endpoint.json"))?;
        Ok(KnownReadLocation {
            project: project.to_owned(),
            directory,
            namespaces: self.namespaces.clone(),
        })
    }

    /// Normal discovery never opens or requires owner-wide inventory. Failure
    /// to admit an intended cache is not a complete empty namespace scan.
    pub fn discovery_view(&self, include_hidden: bool) -> io::Result<RegistryView> {
        if let Some(error) = &self.cache_error {
            return Err(io::Error::other(error.clone()));
        }
        let inventory = include_hidden
            .then(|| self.roots.inventory_root())
            .transpose()?;
        RegistryView::open(&self.namespaces, inventory)
    }

    /// Mutating admission is reserved for actual publication. At least one
    /// namespace plus the owner inventory are required for a new native host.
    pub fn publication_location(&self) -> io::Result<EndpointLocation> {
        if self.namespaces.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "native host requires a usable namespace root",
            ));
        }
        let inventory = self.roots.inventory_root()?;
        validate_state_separation(&self.state, std::slice::from_ref(&inventory))?;
        let registries = RegistrySet::with_inventory(&self.namespaces, Some(inventory))?;
        EndpointLocation::new(&self.project, self.endpoint.clone(), registries)
    }

    /// Bounded, lossless and role/order-sensitive. This detects a changed child
    /// fallback/default before publication; it grants no filesystem authority.
    pub fn fingerprint(&self) -> io::Result<String> {
        let inventory = self.roots.inventory_root()?;
        let mut bytes = b"runyte-windows-layout-v1\0".to_vec();
        for (role, path) in [
            (1, &self.project),
            (2, &self.state),
            (3, &self.endpoint),
            (4, &self.names),
            (5, &inventory),
        ] {
            fingerprint_path(&mut bytes, role, path)?;
        }
        bytes.push(self.namespaces.len() as u8);
        for path in &self.namespaces {
            fingerprint_path(&mut bytes, 6, path)?;
        }
        if let Some(path) = &self.cache {
            fingerprint_path(&mut bytes, 7, path)?;
        }
        Ok(crate::hash::sha256_hex(&bytes))
    }

    /// Every dedicated startup builder applies these set/remove operations.
    /// The caller also supplies its explicit config and config environment.
    pub fn detached_environment(&self) -> io::Result<Vec<(OsString, Option<OsString>)>> {
        Ok(vec![
            (
                "XDG_RUNTIME_DIR".into(),
                self.runtime
                    .as_ref()
                    .map(|path| path.as_os_str().to_owned()),
            ),
            (
                "XDG_CACHE_HOME".into(),
                self.roots
                    .cache_home
                    .as_ref()
                    .filter(|path| path.is_absolute())
                    .map(|path| path.as_os_str().to_owned()),
            ),
            (
                "RUNYTE_ALL_HOSTS_DIR".into(),
                Some(self.roots.inventory_root()?.into_os_string()),
            ),
            (EXPECTED_LAYOUT_ENV.into(), Some(self.fingerprint()?.into())),
        ])
    }

    /// Main must pass true only for its dedicated --serve --detached-host
    /// launch, never ordinary launches which may inherit another host's marker.
    /// No environment is read here; the caller captures the expected value.
    pub fn verify_detached_layout(
        &self,
        dedicated_detached_host: bool,
        expected: Option<&OsStr>,
    ) -> io::Result<()> {
        if dedicated_detached_host
            && let Some(expected) = expected
            && expected != OsStr::new(&self.fingerprint()?)
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "detached host configured layout changed before publication",
            ));
        }
        Ok(())
    }
}

fn fingerprint_path(bytes: &mut Vec<u8>, role: u8, path: &Path) -> io::Result<()> {
    validate_path(path)?;
    let encoded = encode_path(path);
    bytes.push(role);
    bytes.extend_from_slice(&(encoded.len() as u32).to_le_bytes());
    bytes.extend_from_slice(&encoded);
    Ok(())
}

fn validate_path(path: &Path) -> io::Result<()> {
    let mut components = path.components();
    if !matches!(components.next(), Some(Component::Prefix(prefix)) if matches!(prefix.kind(), Prefix::Disk(_) | Prefix::VerbatimDisk(_)))
        || !matches!(components.next(), Some(Component::RootDir))
        || path.as_os_str().encode_wide().any(|unit| unit == 0)
        || encode_path(path).len() > MAX_PERSISTED_PATH_BYTES
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "native locations require bounded absolute local paths without NUL units",
        ));
    }
    for component in components {
        let Component::Normal(name) = component else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "native locations require ordinary path components",
            ));
        };
        // Path identity/fingerprints preserve native units, including lone
        // surrogates. Private-storage admission applies its own stricter leaf
        // policy when a location is actually opened.
        if name.to_str().is_some() {
            crate::windows_fs::validate_relative(Path::new(name))?;
        }
    }
    Ok(())
}

fn admit_cache(path: &Path) -> io::Result<()> {
    validate_path(path)?;
    match Directory::open_existing(path, true) {
        Ok(_) => return Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    // Existing ancestors need not be private (for example the OS account
    // directory); the new Runyte leaf will be private on publication admission.
    for parent in path.ancestors().skip(1) {
        match Directory::open_existing(parent, false) {
            Ok(_) => return Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::NotFound,
        "cache has no admitted local ancestor",
    ))
}

// A missing leaf must not turn Windows aliases into different reserved roots.
// Resolve the nearest existing ancestor and compare native components without
// regard to case. Conservative overlap rejection also protects optional NTFS
// case-sensitive directories; it never grants access based on name equality.
fn validate_state_separation(state: &Path, reserved: &[PathBuf]) -> io::Result<()> {
    let state = comparison_components(state)?;
    for reserved in reserved {
        let reserved = comparison_components(reserved)?;
        if state
            .iter()
            .zip(&reserved)
            .all(|(left, right)| crate::windows_fs::compare_names(left, right).is_eq())
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "workspace state overlaps reserved per-user storage",
            ));
        }
    }
    Ok(())
}

fn comparison_components(path: &Path) -> io::Result<Vec<Vec<u16>>> {
    validate_path(path)?;
    let mut missing = Vec::new();
    let mut ancestor = path;
    let mut resolved = loop {
        match ancestor.canonicalize() {
            Ok(path) => break path,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                missing.push(ancestor.file_name().ok_or(error)?.to_owned());
                ancestor = ancestor.parent().ok_or_else(|| {
                    io::Error::new(io::ErrorKind::NotFound, "location has no existing ancestor")
                })?;
            }
            Err(error) => return Err(error),
        }
    };
    for component in missing.into_iter().rev() {
        resolved.push(component);
    }
    validate_path(&resolved)?;
    Ok(resolved
        .components()
        .filter_map(|component| match component {
            Component::Prefix(prefix) => match prefix.kind() {
                Prefix::Disk(drive) | Prefix::VerbatimDisk(drive) => Some(vec![drive as u16]),
                _ => unreachable!("validated local path"),
            },
            Component::Normal(name) => Some(name.encode_wide().collect()),
            Component::RootDir => None,
            _ => unreachable!("validated native path"),
        })
        .collect())
}

#[cfg(test)]
mod tests;
