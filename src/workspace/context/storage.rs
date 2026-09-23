// SPDX-License-Identifier: MPL-2.0

//! Private, bounded bridge identity, remembered grant, and live-host records.
//! Opening a store is an explicit opt-in; discovery never attaches to a host.

use super::wire::Scope;
use crate::private_storage::Directory;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::{
    collections::BTreeSet,
    ffi::{OsStr, OsString},
    fmt,
    fs::File,
    io::{self, Read, Write},
    path::{Path, PathBuf},
};

const RECORD_LIMIT: usize = 64 * 1024;
const DISCOVERY_LIMIT: usize = 512;

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Identity {
    name: String,
    credential: String,
}
impl fmt::Debug for Identity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Identity")
            .field("name", &self.name)
            .finish_non_exhaustive()
    }
}
impl Identity {
    pub fn name(&self) -> &str {
        &self.name
    }
    pub fn credential(&self) -> &str {
        &self.credential
    }
    pub fn fingerprint(&self) -> String {
        crate::hash::sha256_hex(self.credential.as_bytes())
    }
}

#[cfg(any(unix, windows))]
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HostMode {
    Persistent,
    Standalone,
}

#[cfg(any(unix, windows))]
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Registration {
    pub root: PathBuf,
    pub workspace_id: String,
    pub host_incarnation: String,
    pub endpoint: PathBuf,
    pub mode: HostMode,
    pub pid: u32,
    #[cfg(windows)]
    pub creation_time: u64,
    pub environment: String,
}

#[cfg(windows)]
#[derive(Debug)]
pub(crate) struct Publication {
    storage: std::sync::Arc<Storage>,
    name: String,
    file: Option<File>,
}

#[cfg(windows)]
impl Publication {
    pub(crate) fn retire(mut self) -> io::Result<()> {
        let removed = self.storage.directory.remove_owned(
            OsStr::new(&self.name),
            self.file.as_ref().expect("publication owns its file"),
        );
        match removed {
            Ok(()) => {
                self.file.take();
                self.storage.directory.sync()
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error),
        }
    }
}

#[cfg(windows)]
impl Drop for Publication {
    fn drop(&mut self) {
        if let Some(file) = &self.file {
            let _ = self
                .storage
                .directory
                .remove_owned(OsStr::new(&self.name), file);
            let _ = self.storage.directory.sync();
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Grant {
    root: Vec<u8>,
    identity: String,
    scopes: BTreeSet<Scope>,
}

#[derive(Debug)]
pub struct Storage {
    root: PathBuf,
    directory: Directory,
    read_only: bool,
}

#[cfg(windows)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StorageLocation {
    root: PathBuf,
    anchor: PathBuf,
}

#[cfg(windows)]
impl StorageLocation {
    fn anchored(root: PathBuf, anchor: PathBuf) -> Option<Self> {
        if !root.is_absolute() || !anchor.is_absolute() || !anchor.is_dir() {
            return None;
        }
        let relative = root.strip_prefix(&anchor).ok()?;
        if relative.as_os_str().is_empty()
            || relative
                .components()
                .any(|component| !matches!(component, std::path::Component::Normal(_)))
        {
            return None;
        }
        Some(Self { root, anchor })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }
}

#[cfg(windows)]
fn select_default_location(
    configured: Option<OsString>,
    local_app_data: Option<PathBuf>,
) -> Option<StorageLocation> {
    match configured {
        Some(path) => {
            let root = PathBuf::from(path);
            let anchor = root.parent()?.to_owned();
            StorageLocation::anchored(root, anchor)
        }
        None => {
            let anchor = local_app_data?;
            StorageLocation::anchored(anchor.join("runyte").join("context"), anchor)
        }
    }
}

#[cfg(unix)]
struct StorageLock {
    _file: File,
}

#[cfg(windows)]
struct StorageLock {
    file: File,
}

#[cfg(windows)]
impl StorageLock {
    fn acquire(file: File) -> io::Result<Self> {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::{
            Foundation::ERROR_LOCK_VIOLATION,
            Storage::FileSystem::{LOCKFILE_EXCLUSIVE_LOCK, LOCKFILE_FAIL_IMMEDIATELY, LockFileEx},
            System::IO::OVERLAPPED,
        };
        let mut offset: OVERLAPPED = unsafe { std::mem::zeroed() };
        if unsafe {
            LockFileEx(
                file.as_raw_handle(),
                LOCKFILE_EXCLUSIVE_LOCK | LOCKFILE_FAIL_IMMEDIATELY,
                0,
                1,
                0,
                &mut offset,
            )
        } == 0
        {
            let error = io::Error::last_os_error();
            return Err(
                if error.raw_os_error() == Some(ERROR_LOCK_VIOLATION as i32) {
                    io::Error::new(io::ErrorKind::WouldBlock, "context storage is busy")
                } else {
                    error
                },
            );
        }
        Ok(Self { file })
    }
}

#[cfg(windows)]
impl Drop for StorageLock {
    fn drop(&mut self) {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::{Storage::FileSystem::UnlockFileEx, System::IO::OVERLAPPED};
        let mut offset: OVERLAPPED = unsafe { std::mem::zeroed() };
        let _ = unsafe { UnlockFileEx(self.file.as_raw_handle(), 0, 1, 0, &mut offset) };
    }
}

impl Storage {
    #[cfg(unix)]
    pub fn default_root() -> Option<PathBuf> {
        let configured = std::env::var_os("RUNYTE_CONTEXT_HOME");
        if let Some(path) = configured {
            let path = PathBuf::from(path);
            return path.is_absolute().then_some(path);
        }
        if cfg!(test) {
            return None;
        }
        let home = crate::user_paths::system_home_directory()?;
        #[cfg(target_os = "macos")]
        let path = home.join("Library/Caches/runyte/context");
        #[cfg(not(target_os = "macos"))]
        let path = home.join(".cache/runyte/context");
        Some(path)
    }

    /// Resolves the Windows context root together with its stable durability
    /// anchor. The default is anchored at OS-resolved LocalAppData. An explicit
    /// RUNYTE_CONTEXT_HOME must be absolute and have an already-existing
    /// immediate parent; retries never promote a descendant left by failure.
    #[cfg(windows)]
    pub fn default_location() -> Option<StorageLocation> {
        let configured = std::env::var_os("RUNYTE_CONTEXT_HOME");
        if configured.is_some() {
            return select_default_location(configured, None);
        }
        if cfg!(test) {
            return None;
        }
        select_default_location(None, crate::user_paths::system_local_app_data_directory())
    }

    pub fn new(root: PathBuf) -> io::Result<Self> {
        if !root.is_absolute() {
            return Err(invalid("context storage requires an absolute path"));
        }
        #[cfg(unix)]
        let directory = Directory::open_durable(&root, true)?;
        #[cfg(windows)]
        let directory = Directory::open_durable_beneath(
            root.parent()
                .filter(|parent| parent.is_dir())
                .ok_or_else(|| invalid("RUNYTE_CONTEXT_HOME parent must already exist"))?,
            Path::new(
                root.file_name()
                    .ok_or_else(|| invalid("context storage requires a directory leaf"))?,
            ),
        )?;
        let root = root.canonicalize()?;
        Ok(Self {
            root,
            directory,
            read_only: false,
        })
    }

    #[cfg(windows)]
    pub fn open_location(location: StorageLocation) -> io::Result<Self> {
        let relative = location
            .root
            .strip_prefix(&location.anchor)
            .map_err(|_| invalid("context storage is outside its durable anchor"))?;
        let directory = Directory::open_durable_beneath(&location.anchor, relative)?;
        let root = location.root.canonicalize()?;
        Ok(Self {
            root,
            directory,
            read_only: false,
        })
    }
    /// Existing stores only; missing or insecure paths fail without writes.
    pub fn open_existing(root: PathBuf) -> io::Result<Self> {
        if !root.is_absolute() {
            return Err(invalid("context storage requires an absolute path"));
        }
        let directory = Directory::open_existing(&root, true)?;
        let root = root.canonicalize()?;
        Ok(Self {
            root,
            directory,
            read_only: true,
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn load_identity(&self, name: &str) -> io::Result<Option<Identity>> {
        let filename = identity_file(name)?;
        let Some(identity): Option<Identity> = self.read(&filename)? else {
            return Ok(None);
        };
        if identity.name != name || !valid_hex(&identity.credential) {
            return Err(invalid("invalid context identity record"));
        }
        Ok(Some(identity))
    }

    pub fn identities(&self) -> io::Result<Vec<Identity>> {
        let mut identities = Vec::new();
        for name in self.entries()? {
            let Some(name) = name.to_str() else {
                continue;
            };
            let identity_name = if name == "identity.json" {
                "agent"
            } else {
                let Some(name) = name
                    .strip_prefix("identity-")
                    .and_then(|name| name.strip_suffix(".json"))
                else {
                    continue;
                };
                name
            };
            if let Ok(Some(identity)) = self.load_identity(identity_name) {
                identities.push(identity);
                if identities.len() > 32 {
                    return Err(invalid("context identity inventory exceeds its limit"));
                }
            }
        }
        identities.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(identities)
    }

    pub fn identity(&self, name: &str) -> io::Result<Identity> {
        let filename = identity_file(name)?;
        let _lock = self.lock()?;
        if let Some(identity) = self.load_identity(name)? {
            return Ok(identity);
        }
        if self.identities()?.len() >= 32 {
            return Err(invalid("context identity inventory exceeds its limit"));
        }
        let identity = Identity {
            name: name.to_owned(),
            credential: random_token()?,
        };
        self.write_locked(&filename, &identity)?;
        Ok(identity)
    }

    pub fn grant(
        &self,
        root: &Path,
        identity: &Identity,
        scopes: BTreeSet<Scope>,
    ) -> io::Result<()> {
        let root = self.workspace_root(root)?;
        super::wire::validate_scopes(&scopes)
            .map_err(|_| invalid("invalid context grant scopes"))?;
        let record = Grant {
            root: path_bytes(&root),
            identity: identity.fingerprint(),
            scopes,
        };
        self.write(&grant_file(&record.root, &record.identity), &record)
    }

    pub fn scopes(&self, root: &Path, identity: &Identity) -> io::Result<BTreeSet<Scope>> {
        let root = path_bytes(&self.workspace_root(root)?);
        let fingerprint = identity.fingerprint();
        let Some(record): Option<Grant> = self.read(&grant_file(&root, &fingerprint))? else {
            return Ok(BTreeSet::new());
        };
        if record.root != root || record.identity != fingerprint {
            return Err(invalid(
                "context grant names a different workspace or identity",
            ));
        }
        super::wire::validate_scopes(&record.scopes)
            .map_err(|_| invalid("invalid context grant scopes"))?;
        Ok(record.scopes)
    }

    pub fn revoke(&self, root: &Path, identity: &Identity) -> io::Result<()> {
        let root = self.workspace_root(root)?;
        self.remove(&grant_file(&path_bytes(&root), &identity.fingerprint()))
    }

    fn workspace_root(&self, root: &Path) -> io::Result<PathBuf> {
        let root = root.canonicalize()?;
        if !root.is_dir() || self.root.starts_with(&root) {
            return Err(invalid("context storage must be outside the workspace"));
        }
        Ok(root)
    }

    #[cfg(unix)]
    pub fn socket_path(&self, incarnation: &str) -> io::Result<PathBuf> {
        if !valid_hex(incarnation) {
            return Err(invalid("invalid host incarnation"));
        }
        let path = self.root.join(format!("h-{}.sock", &incarnation[..32]));
        // macOS sockaddr_un.sun_path has 104 bytes, including the terminator.
        if path_bytes(&path).len() >= 104 {
            return Err(invalid("context socket path is too long"));
        }
        Ok(path)
    }

    #[cfg(windows)]
    pub fn socket_path(&self, incarnation: &str) -> io::Result<PathBuf> {
        if !valid_hex(incarnation) {
            return Err(invalid("invalid host incarnation"));
        }
        Ok(PathBuf::from(format!(
            r"\\.\pipe\runyte-context-v1-{incarnation}"
        )))
    }

    #[cfg(unix)]
    pub fn register(&self, registration: &Registration) -> io::Result<()> {
        self.validate_registration(registration)?;
        self.write(
            &registration_file(&registration.host_incarnation),
            registration,
        )
    }

    #[cfg(windows)]
    pub(crate) fn register_owned(
        self: &std::sync::Arc<Self>,
        registration: &Registration,
    ) -> io::Result<Publication> {
        self.require_writable()?;
        self.validate_registration(registration)?;
        let _lock = self.lock()?;
        let name = registration_file(&registration.host_incarnation);
        let entries = self.entries()?;
        if entries.len() >= DISCOVERY_LIMIT
            && !entries.iter().any(|entry| entry == OsStr::new(&name))
        {
            return Err(invalid("context storage inventory exceeds its limit"));
        }
        let bytes = serde_json::to_vec(registration)
            .map_err(|_| invalid("invalid context storage record"))?;
        if bytes.len() > RECORD_LIMIT {
            return Err(invalid("context storage record exceeds its size limit"));
        }
        let file = self
            .directory
            .atomic_write_owned(OsStr::new(&name), &bytes)?;
        Ok(Publication {
            storage: self.clone(),
            name,
            file: Some(file),
        })
    }
    #[cfg(unix)]
    pub fn unregister(&self, incarnation: &str) -> io::Result<()> {
        if !valid_hex(incarnation) {
            return Err(invalid("invalid host incarnation"));
        }
        self.remove(&registration_file(incarnation))
    }

    /// File contents are read relative to the pinned directory. Directory
    /// enumeration contributes names only; links, replaced files, and malformed
    /// records cannot redirect an admitted endpoint outside private storage.
    /// Transport must independently authenticate the socket and host incarnation.
    #[cfg(any(unix, windows))]
    pub fn discover(
        &self,
        environment: &str,
        include_hidden: bool,
    ) -> io::Result<Vec<Registration>> {
        if !valid_hex(environment) {
            return Err(invalid("invalid environment fingerprint"));
        }
        let mut records = Vec::new();
        for name in self.entries()? {
            let Some(name) = name.to_str() else {
                continue;
            };
            if !name.starts_with("host-") || !name.ends_with(".json") {
                continue;
            }
            let Ok(Some(record)) = self.read::<Registration>(name) else {
                continue;
            };
            if self.validate_registration_shape(&record).is_err()
                || name != registration_file(&record.host_incarnation)
            {
                continue;
            }
            if include_hidden || record.environment == environment {
                records.push(record);
            }
        }
        records.sort_by(|a, b| a.host_incarnation.cmp(&b.host_incarnation));
        Ok(records)
    }

    #[cfg(any(unix, windows))]
    fn validate_registration(&self, record: &Registration) -> io::Result<()> {
        if path_bytes(&record.root) != path_bytes(&self.workspace_root(&record.root)?) {
            return Err(invalid("context registration root is not canonical"));
        }
        self.validate_registration_shape(record)
    }

    // Discovery does not touch project paths: a stale root can be missing or
    // on a disconnected network mount. The bounded live probe confirms identity.
    #[cfg(any(unix, windows))]
    fn validate_registration_shape(&self, record: &Registration) -> io::Result<()> {
        #[cfg(windows)]
        let native_identity_valid = record.creation_time != 0
            && record.root.to_str().is_some()
            && record.endpoint.to_str().is_some();
        #[cfg(unix)]
        let native_identity_valid = true;
        if !valid_hex(&record.host_incarnation)
            || !valid_hex(&record.environment)
            || record.pid == 0
            || !native_identity_valid
            || !record.root.is_absolute()
            || record
                .root
                .components()
                .any(|part| matches!(part, std::path::Component::ParentDir))
            || self.root.starts_with(&record.root)
            || record.workspace_id != crate::workspace::identity::workspace_id(&record.root)
            || record.endpoint != self.socket_path(&record.host_incarnation)?
        {
            return Err(invalid("invalid context host registration"));
        }
        Ok(())
    }

    fn entries(&self) -> io::Result<Vec<OsString>> {
        #[cfg(unix)]
        let (entries, truncated) = {
            let entries = std::fs::read_dir(&self.root)?
                .take(DISCOVERY_LIMIT + 1)
                .map(|entry| entry.map(|entry| entry.file_name()))
                .collect::<io::Result<Vec<_>>>()?;
            let truncated = entries.len() > DISCOVERY_LIMIT;
            (entries, truncated)
        };
        #[cfg(windows)]
        let (entries, truncated) = self.directory.entries(DISCOVERY_LIMIT)?;
        if truncated {
            return Err(invalid("context storage inventory exceeds its limit"));
        }
        Ok(entries)
    }

    fn read<T: DeserializeOwned>(&self, name: &str) -> io::Result<Option<T>> {
        let read = if self.read_only {
            (|| {
                #[cfg(unix)]
                use std::os::unix::fs::MetadataExt;
                let file = self.directory.open_read(OsStr::new(name))?;
                #[cfg(unix)]
                if file.metadata()?.mode() & 0o077 != 0 {
                    return Err(invalid("context storage record is not private"));
                }
                let mut bytes = Vec::new();
                file.take((RECORD_LIMIT + 1) as u64)
                    .read_to_end(&mut bytes)?;
                if bytes.len() > RECORD_LIMIT {
                    return Err(invalid("context storage record exceeds its size limit"));
                }
                Ok(bytes)
            })()
        } else {
            self.directory.read(OsStr::new(name), RECORD_LIMIT)
        };
        let bytes = match read {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        };
        serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|_| invalid("invalid context storage record"))
    }
    fn write(&self, name: &str, record: &impl Serialize) -> io::Result<()> {
        self.require_writable()?;
        let _lock = self.lock()?;
        self.write_locked(name, record)
    }
    fn write_locked(&self, name: &str, record: &impl Serialize) -> io::Result<()> {
        let entries = self.entries()?;
        if entries.len() >= DISCOVERY_LIMIT
            && !entries.iter().any(|entry| entry == OsStr::new(name))
        {
            return Err(invalid("context storage inventory exceeds its limit"));
        }
        let bytes =
            serde_json::to_vec(record).map_err(|_| invalid("invalid context storage record"))?;
        if bytes.len() > RECORD_LIMIT {
            return Err(invalid("context storage record exceeds its size limit"));
        }
        self.directory.atomic_write(OsStr::new(name), &bytes)
    }
    fn remove(&self, name: &str) -> io::Result<()> {
        self.require_writable()?;
        let _lock = self.lock()?;
        #[cfg(unix)]
        let removed = self.directory.remove(OsStr::new(name));
        #[cfg(windows)]
        let removed = (|| {
            let file = self.directory.open_read(OsStr::new(name))?;
            self.directory.remove_owned(OsStr::new(name), &file)
        })();
        match removed {
            Ok(()) => self.directory.sync(),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error),
        }
    }
    fn require_writable(&self) -> io::Result<()> {
        if self.read_only {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "context storage was opened read-only",
            ));
        }
        Ok(())
    }
    fn lock(&self) -> io::Result<StorageLock> {
        self.require_writable()?;
        let file = self.directory.append(OsStr::new("identity.lock"))?;
        #[cfg(unix)]
        {
            use std::os::fd::AsRawFd;
            // Inventory admission and identity creation share this stable inode.
            // SAFETY: file owns a valid descriptor for the duration of the lock.
            while unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) } != 0 {
                let error = io::Error::last_os_error();
                if error.kind() != io::ErrorKind::Interrupted {
                    return Err(error);
                }
            }
            Ok(StorageLock { _file: file })
        }
        #[cfg(windows)]
        {
            StorageLock::acquire(file)
        }
    }
}

pub fn random_token() -> io::Result<String> {
    let mut bytes = [0_u8; 32];
    #[cfg(unix)]
    File::open("/dev/urandom")?.read_exact(&mut bytes)?;
    #[cfg(windows)]
    {
        use windows_sys::Win32::Security::Cryptography::{
            BCRYPT_USE_SYSTEM_PREFERRED_RNG, BCryptGenRandom,
        };
        let status = unsafe {
            BCryptGenRandom(
                std::ptr::null_mut(),
                bytes.as_mut_ptr(),
                bytes.len() as u32,
                BCRYPT_USE_SYSTEM_PREFERRED_RNG,
            )
        };
        if status < 0 {
            return Err(io::Error::other("system random generator failed"));
        }
    }
    let mut encoded = Vec::with_capacity(64);
    for byte in bytes {
        write!(&mut encoded, "{byte:02x}")?;
    }
    String::from_utf8(encoded).map_err(|_| invalid("random identity encoding failed"))
}

pub fn environment_fingerprint() -> String {
    let paths = [
        std::env::var_os("XDG_RUNTIME_DIR"),
        std::env::var_os("XDG_CACHE_HOME"),
    ];
    let mut bytes = Vec::new();
    for path in paths {
        match path {
            Some(path) => {
                bytes.push(1);
                let path = path_bytes(Path::new(&path));
                bytes.extend_from_slice(&(path.len() as u64).to_le_bytes());
                bytes.extend(path);
            }
            None => bytes.push(0),
        }
    }
    crate::hash::sha256_hex(&bytes)
}
fn path_bytes(path: &Path) -> Vec<u8> {
    crate::native_path::encode_path(path)
}
fn identity_file(name: &str) -> io::Result<String> {
    if name.is_empty()
        || name.len() > 64
        || !name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return Err(invalid(
            "identity names use 1 to 64 ASCII letters, digits, underscores or hyphens",
        ));
    }
    Ok(if name == "agent" {
        "identity.json".to_owned()
    } else {
        format!("identity-{name}.json")
    })
}
fn grant_file(root: &[u8], identity: &str) -> String {
    let mut bytes = root.to_vec();
    bytes.push(0);
    bytes.extend_from_slice(identity.as_bytes());
    format!("grant-{}.json", crate::hash::sha256_hex(&bytes))
}
#[cfg(any(unix, windows))]
fn registration_file(incarnation: &str) -> String {
    format!("host-{incarnation}.json")
}
fn valid_hex(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

#[cfg(all(test, unix))]
#[path = "tests/storage.rs"]
mod tests;

#[cfg(all(test, windows))]
#[path = "tests/storage_windows.rs"]
mod tests;
