// SPDX-License-Identifier: MPL-2.0

//! Versioned, bounded local transport for bundled Runyte clients.

use std::{
    fs::{self, OpenOptions},
    io::{self, Read as _},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use tokio::{
    io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader},
    net::{UnixListener, UnixStream, unix::OwnedReadHalf, unix::OwnedWriteHalf},
    sync::{Semaphore, mpsc, oneshot},
};

use crate::app::FrameGeometry;

pub use crate::protocol::{
    CLIENT_VERSION, ClientKind, ClientRequest, ClientRole, FeatureGroup, HostResponse,
    MAX_FEATURE_GROUPS, TransportChange, decode_path, encode_path,
};

pub const PROTOCOL_VERSION: u32 = crate::protocol::VERSION;
const MAX_MESSAGE_BYTES: usize = 8 * 1024 * 1024;
/// Deep enough that an ordinary burst of frames — one per keystroke, plus
/// overlay redraws — never backs up against a client that is still draining.
/// Reaching this depth means the client has genuinely stopped reading.
const RESPONSE_CAPACITY: usize = 64;
const EVENT_CAPACITY: usize = 64;
/// A local host normally has one interactive connection and a handful of
/// short-lived control connections. Bounding accepted peers prevents a burst
/// of incomplete handshakes from retaining one task and framing buffer each.
const MAX_CONNECTIONS: usize = 16;
/// How long a peer may accept no bytes at all before its connection is
/// abandoned. A peer that has stopped reading must not retain its connection
/// task or keep the host's interactive attachment occupied indefinitely, but
/// the budget measures a stall rather than a whole message: see
/// `write_message_with_timeout`.
const CONNECTION_WRITE_STALL: Duration = Duration::from_secs(2);
const HOST_ID_LENGTH: usize = crate::workspace::WORKSPACE_ID_LENGTH;
pub(crate) const MAX_HOST_NAME_BYTES: usize = 64;
const MAX_METADATA_BYTES: usize = 64 * 1024;
const MAX_STORED_NAME_BYTES: usize = 1024;
pub(super) const MAX_PERSISTED_PATH_BYTES: usize = 4 * 1024;
const MAX_REGISTERED_HOSTS: usize = 1024;
/// How long discovery waits for an endpoint to accept a probe connection. The
/// same bound as registry discovery uses: long enough for a busy host, short
/// enough that listing a directory of workspaces stays interactive.
const PUBLISHED_HOST_PROBE: Duration = Duration::from_millis(250);

#[cfg(unix)]
fn socket_path_capacity() -> usize {
    // `sockaddr_un` is a plain C address struct; its all-zero representation
    // is valid and lets this build read the target platform's `sun_path` size.
    let address = unsafe { std::mem::zeroed::<libc::sockaddr_un>() };
    address.sun_path.len().saturating_sub(1)
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct EndpointMetadata {
    pub protocol: u32,
    pub pid: u32,
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    pub project_root_bytes: Vec<u8>,
    pub socket_bytes: Vec<u8>,
}

/// What an endpoint file publishes about the host holding it, read without a
/// handshake. A host of another protocol version can be described this way but
/// never spoken to, so this carries only what listing and stopping need.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PublishedHost {
    pub id: String,
    pub name: Option<String>,
    pub pid: u32,
    pub protocol: u32,
    pub project_root: PathBuf,
}

impl PublishedHost {
    pub fn speaks_current_protocol(&self) -> bool {
        self.protocol == PROTOCOL_VERSION
    }
}

/// A running host whose protocol this build cannot speak.
///
/// It is a type rather than a message because callers act on it: `--wait` has
/// to tell "cannot speak to this host" apart from "no host is there" before it
/// decides whether starting one is safe, and `--session-stop` has to fall
/// back to stopping it without a handshake. Both decisions once depended on
/// matching the word "incompatible" in the error text.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IncompatibleHost {
    pub protocol: u32,
    pub pid: u32,
    pub project_root: PathBuf,
}

impl std::fmt::Display for IncompatibleHost {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "workspace host protocol {} is incompatible with client protocol {}; \
             process {} is still running it, so stop it with `runyte --session-stop {}`",
            self.protocol,
            PROTOCOL_VERSION,
            self.pid,
            self.project_root.display()
        )
    }
}

impl std::error::Error for IncompatibleHost {}

#[derive(Clone, Debug)]
pub struct LocalEndpoint {
    directory: PathBuf,
    metadata: PathBuf,
    socket: PathBuf,
    project_root: PathBuf,
    id: String,
    registry: Option<PathBuf>,
    secondary_registry: Option<PathBuf>,
    /// Owner-wide discovery used only by explicit all-namespace lifecycle
    /// operations. It never participates in ordinary session selection or
    /// name allocation, so deliberately isolated namespaces stay isolated.
    inventory_registry: InventoryRegistry,
    /// Integration-only ownership carried by explicitly injected endpoints.
    /// Detached children inherit it so abrupt test-runner termination retires
    /// them just like foreground test hosts.
    test_supervisor: Option<u32>,
    name_file: Option<PathBuf>,
    runtime_root: Option<PathBuf>,
}

struct EndpointPublication {
    registry: Option<PathBuf>,
    secondary_registry: Option<PathBuf>,
    inventory_registry: InventoryRegistry,
    test_supervisor: Option<u32>,
    runtime_root: Option<PathBuf>,
}

#[derive(Clone, Debug)]
enum InventoryRegistry {
    Disabled,
    ResolveForPublication,
    Exact(PathBuf),
}

#[derive(Clone, Debug)]
pub struct RegisteredHost {
    pub id: String,
    pub name: Option<String>,
    pub pid: u32,
    pub protocol: u32,
    pub project_root: PathBuf,
    endpoint: LocalEndpoint,
}

impl RegisteredHost {
    pub fn display_id(&self) -> &str {
        &self.id
    }

    pub fn endpoint(&self) -> &LocalEndpoint {
        &self.endpoint
    }

    pub fn speaks_current_protocol(&self) -> bool {
        self.protocol == PROTOCOL_VERSION
    }
}

impl LocalEndpoint {
    /// Discovers the per-workspace endpoint, preferring a valid user runtime
    /// directory and falling back to the configured workspace runtime root.
    pub fn discover(state_root: &Path, project_root: &Path) -> Result<Self> {
        let runtime = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from);
        if let Some(runtime) = runtime
            .as_deref()
            .filter(|runtime| prepare_runtime_root(runtime))
        {
            let runyte = runtime.join("runyte");
            let id = workspace_id(project_root);
            let runtime_registry = runyte.join("hosts");
            let registry = usable_fallback_registry_root();
            return Self::at_directory(
                runyte.join(&id),
                state_root,
                project_root,
                EndpointPublication {
                    registry: registry.clone().or_else(|| Some(runtime_registry.clone())),
                    secondary_registry: registry.map(|_| runtime_registry),
                    inventory_registry: InventoryRegistry::ResolveForPublication,
                    test_supervisor: None,
                    runtime_root: Some(runtime.to_path_buf()),
                },
            );
        }
        Self::at_directory(
            state_root.join("host"),
            state_root,
            project_root,
            EndpointPublication {
                registry: usable_fallback_registry_root(),
                secondary_registry: None,
                inventory_registry: InventoryRegistry::ResolveForPublication,
                test_supervisor: None,
                runtime_root: None,
            },
        )
    }

    /// Discovers the endpoint against an explicit user runtime directory.
    ///
    /// [`Self::discover`] reads `XDG_RUNTIME_DIR`, which tests must not
    /// publish endpoints into. They pass their own private directory here so
    /// the runtime root stays injectable instead of process-global.
    pub fn discover_with_runtime(
        state_root: &Path,
        project_root: &Path,
        runtime: Option<&Path>,
    ) -> Result<Self> {
        if let Some(runtime) = runtime.filter(|runtime| prepare_runtime_root(runtime)) {
            let runyte = runtime.join("runyte");
            let id = workspace_id(project_root);
            return Self::at_directory(
                runyte.join(&id),
                state_root,
                project_root,
                EndpointPublication {
                    registry: Some(runyte.join("hosts")),
                    secondary_registry: None,
                    inventory_registry: InventoryRegistry::Exact(runyte.join("all-hosts")),
                    test_supervisor: Some(std::process::id()),
                    runtime_root: Some(runtime.to_path_buf()),
                },
            );
        }
        Self::new(state_root, project_root)
    }

    pub fn new(state_root: &Path, project_root: &Path) -> Result<Self> {
        Self::at_directory(
            state_root.join("host"),
            state_root,
            project_root,
            EndpointPublication {
                registry: None,
                secondary_registry: None,
                inventory_registry: InventoryRegistry::Disabled,
                test_supervisor: None,
                runtime_root: None,
            },
        )
    }

    fn at_directory(
        directory: PathBuf,
        state_root: &Path,
        project_root: &Path,
        publication: EndpointPublication,
    ) -> Result<Self> {
        let socket = directory.join("workspace.sock");
        let id = workspace_id(project_root);
        #[cfg(unix)]
        ensure!(
            socket.as_os_str().as_encoded_bytes().len() <= socket_path_capacity(),
            "workspace host socket path is too long: {}; configure a shorter workspace.state path",
            socket.display()
        );
        Ok(Self {
            metadata: directory.join("endpoint.json"),
            directory,
            socket,
            project_root: project_root.to_path_buf(),
            name_file: Some(state_root.join("host-names").join(format!("{id}.json"))),
            id,
            registry: publication.registry,
            secondary_registry: publication.secondary_registry,
            inventory_registry: publication.inventory_registry,
            test_supervisor: publication.test_supervisor,
            runtime_root: publication.runtime_root,
        })
    }

    fn from_registered(metadata: &EndpointMetadata, registration: &Path) -> Result<Self> {
        let project_root = decode_path(metadata.project_root_bytes.clone());
        let socket = decode_path(metadata.socket_bytes.clone());
        ensure!(
            socket.is_absolute(),
            "registered host socket is not absolute"
        );
        let directory = socket
            .parent()
            .context("registered host socket has no parent directory")?
            .to_path_buf();
        ensure!(
            metadata.id == workspace_id(&project_root),
            "registered host identity does not match its project directory"
        );
        let runtime_root = directory
            .parent()
            .filter(|runyte| runyte.file_name().is_some_and(|name| name == "runyte"))
            .filter(|_| {
                directory
                    .file_name()
                    .is_some_and(|name| name == metadata.id.as_str())
            })
            .and_then(Path::parent)
            .map(Path::to_path_buf);
        let registry = registration
            .parent()
            .context("host registration has no registry directory")?
            .to_path_buf();
        let inventory_row = registration
            .file_name()
            .is_some_and(|name| name == inventory_registration_file_name(metadata).as_str());
        Ok(Self {
            metadata: directory.join("endpoint.json"),
            directory,
            socket,
            project_root,
            id: metadata.id.clone(),
            registry: (!inventory_row).then_some(registry.clone()),
            secondary_registry: None,
            inventory_registry: if inventory_row {
                InventoryRegistry::Exact(registry)
            } else {
                // Reconstructed local endpoints are used for discovery and
                // lifecycle requests, never publication. The owning host
                // removes its broad row during graceful shutdown; an abrupt
                // stop is retired by the next explicit broad scan.
                InventoryRegistry::Disabled
            },
            test_supervisor: None,
            name_file: None,
            runtime_root,
        })
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn project_root(&self) -> &Path {
        &self.project_root
    }

    pub fn runtime_root(&self) -> Option<&Path> {
        self.runtime_root.as_deref()
    }

    pub(crate) fn inventory_registry(&self) -> Option<&Path> {
        match &self.inventory_registry {
            InventoryRegistry::Exact(path) => Some(path),
            InventoryRegistry::Disabled | InventoryRegistry::ResolveForPublication => None,
        }
    }

    pub(crate) fn test_supervisor(&self) -> Option<u32> {
        self.test_supervisor
    }

    pub fn socket(&self) -> &Path {
        &self.socket
    }

    pub fn metadata(&self) -> &Path {
        &self.metadata
    }

    /// Reports whether this endpoint has a listener that is accepting
    /// connections. A `false` result is reserved for an absent endpoint or a
    /// conclusively stale socket whose recorded process is no longer alive.
    pub async fn listener_is_live(&self) -> Result<bool> {
        if !self.socket.exists() {
            return Ok(false);
        }
        verify_private(&self.socket, false)?;
        match UnixStream::connect(&self.socket).await {
            Ok(_) => Ok(true),
            Err(error) if is_conclusive_stale(&error) => {
                ensure!(
                    !self.recorded_host_is_alive()?,
                    "workspace host process is still alive but its endpoint is not accepting connections; refusing unsafe stale recovery"
                );
                Ok(false)
            }
            Err(error) => Err(error).with_context(|| {
                format!(
                    "cannot determine whether workspace host {} is healthy; endpoint was left intact",
                    self.socket.display()
                )
            }),
        }
    }

    pub async fn bind(&self) -> Result<UnixListener> {
        self.prepare_directory()?;
        let identity_lock = self.lock_identity()?;
        self.ensure_no_registered_host()?;
        if self.socket.exists() {
            if self.listener_is_live().await? {
                bail!(
                    "a workspace host is already listening at {}",
                    self.socket.display()
                );
            }
            self.remove_stale_unlocked()?;
        }
        // `bind` creates the socket with the ambient umask, so narrowing the
        // umask first means it is never briefly group- or world-accessible.
        // The 0o700 parent directory verified above is the primary barrier;
        // this closes the window behind it.
        let listener = {
            let _umask = UmaskGuard::narrow();
            UnixListener::bind(&self.socket)
                .with_context(|| format!("cannot bind workspace host {}", self.socket.display()))?
        };
        if let Err(error) = set_mode(&self.socket, 0o600) {
            drop(listener);
            let _ = remove_if_exists(&self.socket);
            return Err(error);
        }
        let name = match self.load_stored_name() {
            Ok(name) => name,
            Err(error) => {
                drop(listener);
                let _ = remove_if_exists(&self.socket);
                return Err(error);
            }
        };
        let name_lock = match name.as_deref() {
            Some(_) => match PrivateFileLock::acquire(
                self.registry.as_deref(),
                self.secondary_registry.as_deref(),
                self.name_file.as_deref().and_then(Path::parent),
                ".host-names.lock",
                "session name",
            ) {
                Ok(lock) => Some(lock),
                Err(error) => {
                    drop(listener);
                    let _ = remove_if_exists(&self.socket);
                    return Err(error);
                }
            },
            None => None,
        };
        if let Some(name) = name.as_deref()
            && let Err(error) = ensure_host_name_available(
                name,
                &self.id,
                self.registry.as_deref(),
                self.secondary_registry.as_deref(),
            )
        {
            drop(name_lock);
            drop(listener);
            let _ = remove_if_exists(&self.socket);
            return Err(error);
        }
        let metadata = EndpointMetadata {
            protocol: PROTOCOL_VERSION,
            pid: std::process::id(),
            id: self.id.clone(),
            name,
            project_root_bytes: encode_path(&self.project_root),
            socket_bytes: encode_path(&self.socket),
        };
        if let Err(error) = self.publish_metadata(&metadata) {
            drop(name_lock);
            let _ = self.remove_registrations_if_matches(Some(std::process::id()));
            drop(identity_lock);
            drop(listener);
            let _ = remove_if_exists(&self.socket);
            let _ = remove_if_exists(&self.metadata);
            return Err(error);
        }
        drop(name_lock);
        drop(identity_lock);
        Ok(listener)
    }

    pub fn verify_for_connect(&self) -> Result<EndpointMetadata> {
        let metadata = self.read_metadata_for_connect()?;
        self.verify_metadata_identity(&metadata)?;
        Ok(metadata)
    }

    fn verify_compatible_for_connect(&self) -> Result<EndpointMetadata> {
        let metadata = self.read_metadata_for_connect()?;
        if metadata.protocol != PROTOCOL_VERSION {
            // Liveness decides which refusal this is. A host that is still
            // running owns its endpoint and must not be displaced, so it is
            // reported as incompatible. One that has already exited left
            // nothing but files, and refusing on the protocol alone would make
            // that leftover permanent: no code below this point ever runs to
            // notice the process is gone, so the endpoint could never be
            // cleared or replaced. Reporting it as stale hands it to the
            // ordinary recovery every caller already performs.
            ensure!(
                process_is_alive(metadata.pid)?,
                io::Error::new(
                    io::ErrorKind::NotFound,
                    "workspace host endpoint was left behind by a host that is no longer running",
                )
            );
            return Err(anyhow::Error::new(IncompatibleHost {
                protocol: metadata.protocol,
                pid: metadata.pid,
                project_root: self.project_root.clone(),
            }));
        }
        self.verify_metadata_identity(&metadata)?;
        Ok(metadata)
    }

    /// Reads what this endpoint publishes without requiring a protocol this
    /// build can speak.
    ///
    /// [`Self::verify_compatible_for_connect`] is the connecting client's view
    /// and refuses anything it could not talk to. Discovery needs the opposite:
    /// something has to be able to see a host of another version in order to
    /// list it or stop it, and the host registry cannot, because it only keeps
    /// entries whose identity this build's metadata shape can validate.
    ///
    /// `None` means no host is there: no endpoint, an unreadable one, a
    /// recorded process that has exited, or a socket that no longer accepts
    /// connections.
    pub fn published_host(&self) -> Result<Option<PublishedHost>> {
        if !self.metadata.exists() || !self.socket.exists() {
            return Ok(None);
        }
        let Ok(metadata) = self.read_metadata_for_connect() else {
            return Ok(None);
        };
        if metadata.project_root_bytes != encode_path(&self.project_root)
            || metadata.socket_bytes != encode_path(&self.socket)
            || !process_is_alive(metadata.pid)?
        {
            return Ok(None);
        }
        match probe_unix_socket(&self.socket, PUBLISHED_HOST_PROBE) {
            Ok(Some(true)) => {}
            // A timeout is not proof of death, but it is also not something a
            // listing can report as reachable, so both stay silent here.
            _ => return Ok(None),
        }
        Ok(Some(PublishedHost {
            id: self.id.clone(),
            // Pre-identity metadata carries no name, so the workspace shows
            // under its directory until a current host republishes one.
            name: metadata.name,
            pid: metadata.pid,
            protocol: metadata.protocol,
            project_root: self.project_root.clone(),
        }))
    }

    /// Removes the endpoint left behind by `pid`, which must be the process
    /// this endpoint still records. Anything published by another process is
    /// left alone, so a replacement host that bound between the caller's
    /// decision and this call keeps its endpoint.
    pub fn clear_published_host(&self, pid: u32) -> Result<()> {
        let _identity_lock = self.lock_identity()?;
        if self.metadata.exists() {
            let metadata = self.read_metadata_for_connect()?;
            ensure!(
                metadata.pid == pid,
                "workspace host endpoint now belongs to another process"
            );
        }
        remove_if_exists(&self.socket)?;
        remove_if_exists(&self.metadata)?;
        self.remove_registrations_if_matches(Some(pid))?;
        remove_empty_directory(&self.directory)?;
        Ok(())
    }

    fn read_metadata_for_connect(&self) -> Result<EndpointMetadata> {
        use std::os::unix::fs::FileTypeExt;
        verify_private(&self.directory, true)?;
        verify_private(&self.metadata, false)?;
        verify_private(&self.socket, false)?;
        ensure!(
            fs::symlink_metadata(&self.metadata)?.is_file(),
            "workspace host metadata is not a regular file"
        );
        ensure!(
            fs::symlink_metadata(&self.socket)?.file_type().is_socket(),
            "workspace host endpoint is not a Unix-domain socket"
        );
        let metadata = read_endpoint_metadata(&self.metadata, "host metadata")?;
        Ok(metadata)
    }

    fn verify_metadata_identity(&self, metadata: &EndpointMetadata) -> Result<()> {
        ensure!(metadata.id == self.id, "workspace host identity changed");
        ensure!(
            metadata.project_root_bytes == encode_path(&self.project_root),
            "workspace host metadata belongs to a different project"
        );
        ensure!(
            metadata.socket_bytes == encode_path(&self.socket),
            "workspace host socket identity changed"
        );
        Ok(())
    }

    fn remove_stale_unlocked(&self) -> Result<()> {
        remove_if_exists(&self.socket)?;
        remove_if_exists(&self.metadata)?;
        self.remove_registrations_if_matches(None)?;
        Ok(())
    }

    pub fn cleanup(&self) -> Result<()> {
        let _identity_lock = self.lock_identity()?;
        let pid = std::process::id();
        let owns_endpoint = self.endpoint_metadata_matches(Some(pid))?;
        let owns_registration = self.registrations_match(Some(pid))?;
        if owns_endpoint || owns_registration {
            remove_if_exists(&self.socket)?;
            remove_if_exists(&self.metadata)?;
        }
        self.remove_registrations_if_matches(Some(pid))?;
        remove_empty_directory(&self.directory)?;
        Ok(())
    }

    pub fn rename(&self, name: &str) -> Result<()> {
        validate_host_name(name)?;
        let _name_lock = PrivateFileLock::acquire(
            self.registry.as_deref(),
            self.secondary_registry.as_deref(),
            self.name_file.as_deref().and_then(Path::parent),
            ".host-names.lock",
            "session name",
        )?;
        ensure_host_name_available(
            name,
            &self.id,
            self.registry.as_deref(),
            self.secondary_registry.as_deref(),
        )?;
        let mut metadata = self.verify_for_connect()?;
        let previous_metadata = metadata.clone();
        let previous_name = metadata.name.clone();
        metadata.name = Some(name.to_owned());

        self.write_stored_name(name)?;
        if let Err(error) = self.publish_metadata(&metadata) {
            let _ = self.publish_metadata(&previous_metadata);
            let _ = self.restore_stored_name(previous_name.as_deref());
            return Err(error);
        }
        Ok(())
    }

    /// Persists an automatically allocated name without replacing a name the
    /// person explicitly chose for this workspace.
    pub fn store_name_if_absent(&self, name: &str) -> Result<()> {
        validate_host_name(name)?;
        let _name_lock = PrivateFileLock::acquire(
            self.registry.as_deref(),
            self.secondary_registry.as_deref(),
            self.name_file.as_deref().and_then(Path::parent),
            ".host-names.lock",
            "session name",
        )?;
        if self.load_stored_name()?.is_some() {
            return Ok(());
        }
        ensure_host_name_available(
            name,
            &self.id,
            self.registry.as_deref(),
            self.secondary_registry.as_deref(),
        )?;
        self.write_stored_name(name)
    }

    fn publish_metadata(&self, metadata: &EndpointMetadata) -> Result<()> {
        validate_metadata_fields(metadata)?;
        self.verify_metadata_identity(metadata)?;
        for path in self.registration_paths(metadata)? {
            prepare_private_directory(path.parent().expect("registration has a parent"))?;
            write_json_atomic(&path, metadata)?;
        }
        // Endpoint metadata is the readiness marker used by startup and
        // connection paths. Publish it only after every registry row so that
        // observing a ready endpoint also means a concurrent session listing
        // can discover it. A registry reader that arrives earlier is safe: it
        // already rejects a row until this endpoint metadata exists.
        write_json_atomic(&self.metadata, metadata)?;
        Ok(())
    }

    fn endpoint_metadata_matches(&self, expected_pid: Option<u32>) -> Result<bool> {
        let metadata = match read_endpoint_metadata(&self.metadata, "host metadata") {
            Ok(metadata) => metadata,
            Err(error) if is_not_found(&error) => return Ok(false),
            Err(error) => return Err(error),
        };
        Ok(self.metadata_matches(&metadata, expected_pid))
    }

    fn registrations_match(&self, expected_pid: Option<u32>) -> Result<bool> {
        let metadata = EndpointMetadata {
            protocol: PROTOCOL_VERSION,
            pid: expected_pid.unwrap_or_else(std::process::id),
            id: self.id.clone(),
            name: None,
            project_root_bytes: encode_path(&self.project_root),
            socket_bytes: encode_path(&self.socket),
        };
        for path in self.registration_paths(&metadata)? {
            let metadata = match read_endpoint_metadata(&path, "host registry entry") {
                Ok(metadata) => metadata,
                Err(error) if is_not_found(&error) => continue,
                Err(error) => return Err(error),
            };
            if self.metadata_matches(&metadata, expected_pid) {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn remove_registrations_if_matches(&self, expected_pid: Option<u32>) -> Result<()> {
        let metadata = EndpointMetadata {
            protocol: PROTOCOL_VERSION,
            pid: expected_pid.unwrap_or_else(std::process::id),
            id: self.id.clone(),
            name: None,
            project_root_bytes: encode_path(&self.project_root),
            socket_bytes: encode_path(&self.socket),
        };
        for path in self.registration_paths(&metadata)? {
            let metadata = match read_endpoint_metadata(&path, "host registry entry") {
                Ok(metadata) => metadata,
                Err(error) if is_not_found(&error) => continue,
                Err(error) => return Err(error),
            };
            if self.metadata_matches(&metadata, expected_pid) {
                remove_if_exists(&path)?;
            }
        }
        Ok(())
    }

    fn metadata_matches(&self, metadata: &EndpointMetadata, expected_pid: Option<u32>) -> bool {
        metadata.id == self.id
            && metadata.project_root_bytes == encode_path(&self.project_root)
            && metadata.socket_bytes == encode_path(&self.socket)
            && expected_pid.is_none_or(|pid| metadata.pid == pid)
    }

    fn registration_paths(&self, metadata: &EndpointMetadata) -> Result<Vec<PathBuf>> {
        let mut paths = [self.registry.as_ref(), self.secondary_registry.as_ref()]
            .into_iter()
            .flatten()
            .map(|registry| registry.join(format!("{}.json", self.id)))
            .collect::<Vec<_>>();
        let inventory = match &self.inventory_registry {
            InventoryRegistry::Disabled => None,
            InventoryRegistry::ResolveForPublication => all_hosts_registry_root()?,
            InventoryRegistry::Exact(path) => Some(path.clone()),
        };
        if let Some(inventory) = inventory.as_deref() {
            paths.push(inventory.join(inventory_registration_file_name(metadata)));
        }
        paths.sort();
        paths.dedup();
        Ok(paths)
    }

    fn lock_identity(&self) -> Result<PrivateFileLock> {
        PrivateFileLock::acquire(
            self.registry.as_deref(),
            self.secondary_registry.as_deref(),
            self.name_file.as_deref().and_then(Path::parent),
            &format!(".host-{}.lock", self.id),
            "host identity",
        )
    }

    fn ensure_no_registered_host(&self) -> Result<()> {
        let mut roots = registry_roots_with(self.registry.as_deref());
        if let Some(registry) = self.secondary_registry.as_deref()
            && !roots.iter().any(|root| root == registry)
        {
            roots.push(registry.to_path_buf());
        }
        if let Some(host) = registered_hosts_in(&roots)?
            .into_iter()
            .find(|host| host.id == self.id)
        {
            bail!(
                "a workspace host for {} is already running at {}",
                self.project_root.display(),
                host.endpoint.socket().display()
            );
        }
        Ok(())
    }

    fn load_stored_name(&self) -> Result<Option<String>> {
        let Some(path) = self.name_file.as_ref() else {
            return Ok(None);
        };
        if !path.exists() {
            return Ok(None);
        }
        verify_private(path, false)?;
        let bytes = read_bounded_file(path, MAX_STORED_NAME_BYTES, "stored session name")?;
        let name: String = serde_json::from_slice(&bytes)
            .with_context(|| format!("malformed stored session name {}", path.display()))?;
        validate_host_name(&name)?;
        Ok(Some(name))
    }

    fn write_stored_name(&self, name: &str) -> Result<()> {
        let path = self
            .name_file
            .as_ref()
            .context("selected host has no persistent name location")?;
        let parent = path
            .parent()
            .context("session name location has no parent directory")?;
        prepare_private_directory(parent)?;
        write_json_atomic(path, &name)
    }

    fn restore_stored_name(&self, name: Option<&str>) -> Result<()> {
        match name {
            Some(name) => self.write_stored_name(name),
            None => {
                if let Some(path) = self.name_file.as_ref() {
                    remove_if_exists(path)?;
                }
                Ok(())
            }
        }
    }

    fn prepare_directory(&self) -> Result<()> {
        if self.directory.exists() {
            verify_private(&self.directory, true)?;
        } else {
            if let Some(parent) = self.directory.parent() {
                fs::create_dir_all(parent).with_context(|| {
                    format!("cannot create host directory parent {}", parent.display())
                })?;
            }
            create_private_directory(&self.directory)?;
        }
        Ok(())
    }

    fn recorded_host_is_alive(&self) -> Result<bool> {
        if !self.metadata.exists() {
            return Ok(false);
        }
        verify_private(&self.metadata, false)?;
        let metadata = read_endpoint_metadata(&self.metadata, "host metadata")?;
        process_is_alive(metadata.pid)
    }
}

pub fn registered_hosts() -> Result<Vec<RegisteredHost>> {
    registered_hosts_in(&registry_roots())
}

/// Enumerates every live host that opted into the owner-wide inventory.
///
/// Current-namespace roots are included as a compatibility path for hosts
/// started by an older build which did not publish the additional row.
pub fn registered_hosts_all_namespaces() -> Result<Vec<RegisteredHost>> {
    registered_hosts_in(&all_registry_roots()?)
}

pub(super) fn registered_hosts_in(roots: &[PathBuf]) -> Result<Vec<RegisteredHost>> {
    let mut hosts = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for registry in roots {
        if !registry.exists() {
            continue;
        }
        verify_private(registry, true)?;
        let directory = fs::read_dir(registry)
            .with_context(|| format!("cannot read host registry {}", registry.display()))?;
        let mut entries = Vec::new();
        for entry in directory {
            let entry = entry?;
            if entry
                .path()
                .extension()
                .and_then(|extension| extension.to_str())
                != Some("json")
            {
                continue;
            }
            ensure!(
                entries.len() < MAX_REGISTERED_HOSTS,
                "host registry {} contains more than {MAX_REGISTERED_HOSTS} entries",
                registry.display()
            );
            entries.push(entry);
        }
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            let path = entry.path();
            if verify_private(&path, false).is_err() {
                continue;
            }
            if !entry.file_type().is_ok_and(|kind| kind.is_file()) {
                continue;
            }
            let metadata = match read_endpoint_metadata(&path, "host registry entry") {
                Ok(metadata) => metadata,
                Err(_) => continue,
            };
            if validate_registered_metadata(&metadata, &path).is_err() {
                continue;
            }
            let Ok(endpoint) = LocalEndpoint::from_registered(&metadata, &path) else {
                continue;
            };
            let process_visible = process_is_alive(metadata.pid)?;
            let live_metadata = match endpoint.verify_for_connect() {
                Ok(metadata) => metadata,
                // Publication writes the registry row before endpoint metadata
                // because the latter is the readiness marker. A visible
                // process with a missing endpoint can therefore be in the
                // intentional publication window rather than dead.
                // If the PID is not visible, only a conclusively dead socket
                // permits mutation: another PID namespace can hide a live
                // process from `kill(pid, 0)` while its endpoint remains real.
                Err(error) if is_stale_endpoint_error(&error) => {
                    if !process_visible
                        && matches!(
                            probe_unix_socket(endpoint.socket(), Duration::from_millis(250)),
                            Ok(Some(false))
                        )
                    {
                        remove_registration_if_pid_matches(&path, metadata.pid)?;
                    }
                    continue;
                }
                Err(_) => continue,
            };
            if live_metadata.protocol != metadata.protocol
                || live_metadata.pid != metadata.pid
                || live_metadata.id != metadata.id
                || live_metadata.project_root_bytes != metadata.project_root_bytes
                || live_metadata.socket_bytes != metadata.socket_bytes
            {
                continue;
            }
            match probe_unix_socket(endpoint.socket(), Duration::from_millis(250)) {
                Ok(Some(true)) => {}
                Ok(Some(false)) => {
                    // Once endpoint metadata is complete, a refused or absent
                    // socket is conclusive even if the recorded PID has been
                    // reused by an unrelated process. The publication-window
                    // exception above applies only before that readiness
                    // marker exists.
                    remove_registration_if_pid_matches(&path, metadata.pid)?;
                    continue;
                }
                Ok(None) | Err(_) => continue,
                // A saturated or unresponsive endpoint is omitted without
                // deleting its registration. Timeout is not proof of death.
            }
            if !seen.insert((live_metadata.id.clone(), live_metadata.socket_bytes.clone())) {
                continue;
            }
            hosts.push(RegisteredHost {
                id: live_metadata.id,
                name: live_metadata.name,
                pid: live_metadata.pid,
                protocol: live_metadata.protocol,
                project_root: endpoint.project_root.clone(),
                endpoint,
            });
        }
    }
    hosts.sort_by(|left, right| left.project_root.cmp(&right.project_root));
    Ok(hosts)
}

/// Probes a Unix-domain listener without allowing a saturated accept queue to
/// block registry discovery indefinitely. `None` is a timeout, `Some(false)`
/// is conclusive staleness, and `Some(true)` is a completed connection.
fn probe_unix_socket(path: &Path, timeout: Duration) -> Result<Option<bool>> {
    use std::os::{
        fd::{AsRawFd, FromRawFd, OwnedFd},
        unix::ffi::OsStrExt,
    };

    let raw = unsafe { libc::socket(libc::AF_UNIX, libc::SOCK_STREAM, 0) };
    if raw == -1 {
        return Err(io::Error::last_os_error()).context("cannot create workspace probe socket");
    }
    let socket = unsafe { OwnedFd::from_raw_fd(raw) };
    let status_flags = unsafe { libc::fcntl(socket.as_raw_fd(), libc::F_GETFL) };
    if status_flags == -1 {
        return Err(io::Error::last_os_error()).context("cannot inspect workspace probe socket");
    }
    let result = unsafe {
        libc::fcntl(
            socket.as_raw_fd(),
            libc::F_SETFL,
            status_flags | libc::O_NONBLOCK,
        )
    };
    if result == -1 {
        return Err(io::Error::last_os_error()).context("cannot make workspace probe nonblocking");
    }
    let descriptor_flags = unsafe { libc::fcntl(socket.as_raw_fd(), libc::F_GETFD) };
    if descriptor_flags == -1 {
        return Err(io::Error::last_os_error())
            .context("cannot inspect workspace probe descriptor");
    }
    let result = unsafe {
        libc::fcntl(
            socket.as_raw_fd(),
            libc::F_SETFD,
            descriptor_flags | libc::FD_CLOEXEC,
        )
    };
    if result == -1 {
        return Err(io::Error::last_os_error())
            .context("cannot make workspace probe close-on-exec");
    }
    let mut address = unsafe { std::mem::zeroed::<libc::sockaddr_un>() };
    address.sun_family = libc::AF_UNIX as libc::sa_family_t;
    let bytes = path.as_os_str().as_bytes();
    ensure!(
        bytes.len() <= socket_path_capacity(),
        "workspace host socket path is too long: {}",
        path.display()
    );
    for (target, source) in address.sun_path.iter_mut().zip(bytes.iter().copied()) {
        *target = source as libc::c_char;
    }
    let length = std::mem::offset_of!(libc::sockaddr_un, sun_path) + bytes.len() + 1;
    let result = unsafe {
        libc::connect(
            socket.as_raw_fd(),
            std::ptr::from_ref(&address).cast::<libc::sockaddr>(),
            length as libc::socklen_t,
        )
    };
    if result == 0 {
        return Ok(Some(true));
    }
    let error = io::Error::last_os_error();
    if matches!(
        error.raw_os_error(),
        Some(libc::ENOENT | libc::ECONNREFUSED)
    ) {
        return Ok(Some(false));
    }
    ensure!(
        matches!(error.raw_os_error(), Some(libc::EINPROGRESS | libc::EAGAIN)),
        "cannot probe workspace host {}: {error}",
        path.display()
    );
    let mut descriptor = libc::pollfd {
        fd: socket.as_raw_fd(),
        events: libc::POLLOUT,
        revents: 0,
    };
    let milliseconds = timeout.as_millis().min(libc::c_int::MAX as u128) as libc::c_int;
    let polled = unsafe { libc::poll(&mut descriptor, 1, milliseconds) };
    if polled == 0 {
        return Ok(None);
    }
    if polled == -1 {
        return Err(io::Error::last_os_error()).context("cannot poll workspace probe socket");
    }
    let mut socket_error: libc::c_int = 0;
    let mut size = std::mem::size_of_val(&socket_error) as libc::socklen_t;
    let result = unsafe {
        libc::getsockopt(
            socket.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_ERROR,
            std::ptr::from_mut(&mut socket_error).cast(),
            &mut size,
        )
    };
    if result == -1 {
        return Err(io::Error::last_os_error()).context("cannot inspect workspace probe socket");
    }
    match socket_error {
        0 => Ok(Some(true)),
        libc::ENOENT | libc::ECONNREFUSED => Ok(Some(false)),
        code => Err(io::Error::from_raw_os_error(code))
            .with_context(|| format!("cannot probe workspace host {}", path.display())),
    }
}

/// Removes a stale registry row only when it still names the process the scan
/// inspected. A replacement host may publish the same workspace identity
/// between the liveness check and cleanup.
fn remove_registration_if_pid_matches(path: &Path, expected_pid: u32) -> Result<()> {
    let metadata = match read_endpoint_metadata(path, "host registry entry") {
        Ok(metadata) => metadata,
        Err(error) if is_not_found(&error) => return Ok(()),
        Err(error) => return Err(error),
    };
    if metadata.pid == expected_pid {
        remove_if_exists(path)?;
    }
    Ok(())
}

fn validate_registered_metadata(metadata: &EndpointMetadata, path: &Path) -> Result<()> {
    validate_metadata_fields(metadata)?;
    ensure!(
        metadata.id.len() == HOST_ID_LENGTH
            && metadata.id.bytes().all(|byte| byte.is_ascii_hexdigit()),
        "host registry entry has an invalid ID: {}",
        path.display()
    );
    let expected_file = format!("{}.json", metadata.id);
    let expected_inventory_file = inventory_registration_file_name(metadata);
    ensure!(
        path.file_name().is_some_and(|name| {
            name == expected_file.as_str() || name == expected_inventory_file.as_str()
        }),
        "host registry filename does not match its ID: {}",
        path.display()
    );
    Ok(())
}

fn inventory_registration_file_name(metadata: &EndpointMetadata) -> String {
    let endpoint = &crate::hash::sha256_hex(&metadata.socket_bytes)[..HOST_ID_LENGTH];
    format!("{}-{endpoint}.json", metadata.id)
}

fn validate_metadata_fields(metadata: &EndpointMetadata) -> Result<()> {
    ensure!(
        metadata.pid > 0 && metadata.pid <= libc::pid_t::MAX as u32,
        "host metadata has an invalid PID"
    );
    if !metadata.id.is_empty() {
        ensure!(
            metadata.id.len() == HOST_ID_LENGTH
                && metadata.id.bytes().all(|byte| byte.is_ascii_hexdigit()),
            "host metadata has an invalid ID"
        );
    }
    validate_metadata_path(&metadata.project_root_bytes, "project directory")?;
    validate_metadata_path(&metadata.socket_bytes, "socket path")?;
    let project_root = decode_path(metadata.project_root_bytes.clone());
    let socket = decode_path(metadata.socket_bytes.clone());
    ensure!(
        project_root.is_absolute(),
        "host metadata project directory is not absolute"
    );
    ensure!(socket.is_absolute(), "host metadata socket is not absolute");
    #[cfg(unix)]
    ensure!(
        metadata.socket_bytes.len() <= socket_path_capacity(),
        "host metadata socket path is too long"
    );
    if let Some(name) = metadata.name.as_deref() {
        validate_host_name(name)?;
    }
    Ok(())
}

fn validate_metadata_path(bytes: &[u8], description: &str) -> Result<()> {
    ensure!(
        !bytes.is_empty(),
        "host metadata has an empty {description}"
    );
    ensure!(
        bytes.len() <= MAX_PERSISTED_PATH_BYTES,
        "host metadata {description} exceeds {MAX_PERSISTED_PATH_BYTES} bytes"
    );
    ensure!(
        !bytes.contains(&0),
        "host metadata {description} contains a null byte"
    );
    Ok(())
}

fn read_endpoint_metadata(path: &Path, description: &str) -> Result<EndpointMetadata> {
    let bytes = read_bounded_file(path, MAX_METADATA_BYTES, description)?;
    let metadata: EndpointMetadata = serde_json::from_slice(&bytes)
        .with_context(|| format!("malformed {description} {}", path.display()))?;
    validate_metadata_fields(&metadata)
        .with_context(|| format!("invalid {description} {}", path.display()))?;
    Ok(metadata)
}

fn read_bounded_file(path: &Path, maximum: usize, description: &str) -> Result<Vec<u8>> {
    let file = fs::File::open(path)
        .with_context(|| format!("cannot read {description} {}", path.display()))?;
    let mut bytes = Vec::new();
    file.take(maximum.saturating_add(1) as u64)
        .read_to_end(&mut bytes)
        .with_context(|| format!("cannot read {description} {}", path.display()))?;
    ensure!(
        bytes.len() <= maximum,
        "{description} {} exceeds {maximum} bytes",
        path.display()
    );
    Ok(bytes)
}

fn is_not_found(error: &anyhow::Error) -> bool {
    error
        .chain()
        .filter_map(|cause| cause.downcast_ref::<io::Error>())
        .any(|error| error.kind() == io::ErrorKind::NotFound)
}

pub(super) fn validate_host_name(name: &str) -> Result<()> {
    ensure!(!name.is_empty(), "session name cannot be empty");
    ensure!(
        name == name.trim(),
        "session name cannot start or end with whitespace"
    );
    ensure!(
        name.len() <= MAX_HOST_NAME_BYTES,
        "session name cannot exceed {MAX_HOST_NAME_BYTES} UTF-8 bytes"
    );
    ensure!(
        !name.chars().any(char::is_control),
        "session name cannot contain control characters"
    );
    Ok(())
}

fn ensure_host_name_available(
    name: &str,
    owner: &str,
    registry: Option<&Path>,
    secondary_registry: Option<&Path>,
) -> Result<()> {
    let mut roots = registry_roots_with(registry);
    if let Some(secondary_registry) = secondary_registry
        && !roots.iter().any(|root| root == secondary_registry)
    {
        roots.push(secondary_registry.to_path_buf());
    }
    if let Some(host) = registered_hosts_in(&roots)?
        .into_iter()
        .find(|host| host.id != owner && host.name.as_deref() == Some(name))
    {
        bail!(
            "session name {name:?} is already used by {} ({})",
            host.project_root.display(),
            host.display_id()
        );
    }
    Ok(())
}

/// Serializes registry scans and publications which claim global identities.
///
/// Every normal discovery path shares the fallback user registry, even when
/// its endpoint lives below `XDG_RUNTIME_DIR`, so that registry is the stable
/// lock location across processes with different runtime directories. The
/// endpoint-local location is only a last resort for deliberately unregistered
/// endpoints constructed by tests or embedders.
struct PrivateFileLock(Vec<fs::File>);

impl PrivateFileLock {
    fn acquire(
        registry: Option<&Path>,
        secondary_registry: Option<&Path>,
        local_fallback: Option<&Path>,
        file_name: &str,
        description: &str,
    ) -> Result<Self> {
        use std::os::{fd::AsRawFd, unix::fs::OpenOptionsExt};

        let mut roots = [registry, secondary_registry]
            .into_iter()
            .flatten()
            .map(Path::to_path_buf)
            .collect::<Vec<_>>();
        if roots.is_empty()
            && let Some(local_fallback) = local_fallback
        {
            roots.push(local_fallback.to_path_buf());
        }
        ensure!(
            !roots.is_empty(),
            "{description} locking has no private storage location"
        );
        roots.sort();
        roots.dedup();

        let mut files = Vec::with_capacity(roots.len());
        for root in roots {
            prepare_private_directory(&root)?;
            let path = root.join(file_name);
            let file = OpenOptions::new()
                .create(true)
                .read(true)
                .write(true)
                .mode(0o600)
                .custom_flags(libc::O_NOFOLLOW)
                .open(&path)
                .with_context(|| format!("cannot open {description} lock {}", path.display()))?;
            verify_private(&path, false)?;
            let result = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) };
            if result == -1 {
                return Err(io::Error::last_os_error()).with_context(|| {
                    format!("cannot lock {description} registry {}", path.display())
                });
            }
            files.push(file);
        }
        Ok(Self(files))
    }
}

impl Drop for PrivateFileLock {
    fn drop(&mut self) {
        use std::os::fd::AsRawFd;
        for file in self.0.iter().rev() {
            let _ = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_UN) };
        }
    }
}

/// Derives the transport identity for a canonical workspace project root.
pub(super) fn workspace_id(project_root: &Path) -> String {
    super::identity::workspace_id(project_root)
}

pub(super) fn registry_roots() -> Vec<PathBuf> {
    if cfg!(test) {
        return Vec::new();
    }
    let mut roots = Vec::new();
    if let Some(runtime) = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .filter(|runtime| valid_runtime_directory(runtime))
    {
        roots.push(runtime.join("runyte/hosts"));
    }
    if let Some(fallback) =
        fallback_registry_root().filter(|root| root.exists() && verify_private(root, true).is_ok())
        && !roots.contains(&fallback)
    {
        roots.push(fallback);
    }
    roots
}

/// Registry roots used only when a lifecycle command explicitly requests the
/// broader owner-wide scope.
pub(super) fn all_registry_roots() -> Result<Vec<PathBuf>> {
    let mut roots = registry_roots();
    if let Some(inventory) = all_hosts_registry_root()?
        && !roots.contains(&inventory)
    {
        roots.push(inventory);
    }
    Ok(roots)
}

/// A machine/boot-local state location independent of XDG namespaces.
///
/// Integration tests override it so subprocess tests never inspect or mutate
/// the person's real inventory. Unit tests do not publish outside their
/// temporary endpoints at all.
fn all_hosts_registry_root() -> Result<Option<PathBuf>> {
    if cfg!(test) {
        return Ok(None);
    }
    if let Some(path) = std::env::var_os("RUNYTE_ALL_HOSTS_DIR").map(PathBuf::from)
        && path.is_absolute()
    {
        return Ok(Some(path));
    }
    let home = system_home_directory().context(
        "cannot resolve the operating-system account home required for owner-wide session inventory",
    )?;
    let namespace = boot_namespace()?;
    Ok(Some(all_hosts_registry_root_for_home(&home, &namespace)))
}

fn all_hosts_registry_root_for_home(home: &Path, namespace: &str) -> PathBuf {
    if cfg!(target_os = "macos") {
        home.join("Library/Application Support/Runyte/all-hosts")
            .join(namespace)
    } else {
        home.join(".local/state/runyte/all-hosts").join(namespace)
    }
}

fn boot_namespace() -> Result<String> {
    let identifier = boot_identifier()?;
    ensure!(
        !identifier.is_empty() && identifier.len() <= 1024,
        "operating system returned an invalid boot identifier"
    );
    Ok(crate::hash::sha256_hex(&identifier)[..HOST_ID_LENGTH].to_owned())
}

#[cfg(target_os = "linux")]
fn boot_identifier() -> Result<Vec<u8>> {
    let identifier = fs::read("/proc/sys/kernel/random/boot_id")
        .context("cannot read the Linux boot identity required for owner-wide session inventory")?;
    let identifier = identifier
        .into_iter()
        .filter(|byte| !byte.is_ascii_whitespace())
        .collect::<Vec<_>>();
    ensure!(
        !identifier.is_empty(),
        "Linux returned an empty boot identity"
    );
    Ok(identifier)
}

#[cfg(target_os = "macos")]
fn boot_identifier() -> Result<Vec<u8>> {
    use std::ffi::CString;

    let name = CString::new("kern.bootsessionuuid").expect("static sysctl name has no NUL");
    let mut length = 0_usize;
    // SAFETY: the first `sysctlbyname` call requests only the required output
    // length and supplies no input value.
    let status = unsafe {
        libc::sysctlbyname(
            name.as_ptr(),
            std::ptr::null_mut(),
            &mut length,
            std::ptr::null_mut(),
            0,
        )
    };
    if status == -1 {
        return Err(io::Error::last_os_error()).context(
            "cannot read the macOS boot identity required for owner-wide session inventory",
        );
    }
    ensure!(
        (2..=1024).contains(&length),
        "macOS returned an invalid boot identity length"
    );
    let mut identifier = vec![0_u8; length];
    // SAFETY: `identifier` is writable for `length` bytes and the second call
    // uses the size returned for the same read-only sysctl.
    let status = unsafe {
        libc::sysctlbyname(
            name.as_ptr(),
            identifier.as_mut_ptr().cast(),
            &mut length,
            std::ptr::null_mut(),
            0,
        )
    };
    if status == -1 {
        return Err(io::Error::last_os_error()).context(
            "cannot read the macOS boot identity required for owner-wide session inventory",
        );
    }
    identifier.truncate(length);
    while identifier.last().is_some_and(|byte| *byte == 0) {
        identifier.pop();
    }
    Ok(identifier)
}

/// Reads the account database rather than `$HOME`, which may deliberately be
/// changed alongside XDG variables by a namespace or test harness. The
/// account-owned parent prevents another user from pre-claiming a predictable
/// path in the system temporary directory.
fn system_home_directory() -> Option<PathBuf> {
    use std::{ffi::CStr, os::unix::ffi::OsStringExt};

    // SAFETY: `sysconf` reads one process configuration value and has no
    // pointer preconditions.
    let configured = unsafe { libc::sysconf(libc::_SC_GETPW_R_SIZE_MAX) };
    let mut capacity = if configured > 0 {
        usize::try_from(configured).ok()?
    } else {
        16 * 1024
    }
    .clamp(1024, 1024 * 1024);
    loop {
        let mut record = std::mem::MaybeUninit::<libc::passwd>::uninit();
        let mut result = std::ptr::null_mut();
        let mut storage = vec![0_u8; capacity];
        // SAFETY: `record`, `storage`, and `result` are live writable storage;
        // the buffer length matches the allocation and the UID is valid.
        let status = unsafe {
            libc::getpwuid_r(
                libc::geteuid(),
                record.as_mut_ptr(),
                storage.as_mut_ptr().cast::<libc::c_char>(),
                storage.len(),
                &mut result,
            )
        };
        if status == libc::ERANGE && capacity < 1024 * 1024 {
            capacity = (capacity * 2).min(1024 * 1024);
            continue;
        }
        if status != 0 || result.is_null() {
            return None;
        }
        // SAFETY: a successful `getpwuid_r` initialized `record` and returned
        // its address through `result`.
        if unsafe { (*result).pw_dir.is_null() } {
            return None;
        }
        // SAFETY: the successful lookup placed a NUL-terminated directory
        // string inside `storage`, which remains alive for this copy.
        let directory = unsafe { CStr::from_ptr((*result).pw_dir) };
        let path = PathBuf::from(std::ffi::OsString::from_vec(directory.to_bytes().to_vec()));
        return (path.is_absolute() && !path.as_os_str().is_empty()).then_some(path);
    }
}

fn registry_roots_with(extra: Option<&Path>) -> Vec<PathBuf> {
    let mut roots = registry_roots();
    if let Some(extra) = extra
        && !roots.iter().any(|root| root == extra)
    {
        roots.push(extra.to_path_buf());
    }
    roots
}

fn fallback_registry_root() -> Option<PathBuf> {
    if cfg!(test) {
        return None;
    }
    if let Some(root) = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .filter(|root| root.is_absolute())
    {
        return Some(root.join("runyte/hosts"));
    }
    let home = std::env::var_os("HOME").map(PathBuf::from)?;
    if cfg!(target_os = "macos") {
        Some(home.join("Library/Caches/runyte/hosts"))
    } else {
        Some(home.join(".cache/runyte/hosts"))
    }
}

fn usable_fallback_registry_root() -> Option<PathBuf> {
    use std::os::unix::fs::OpenOptionsExt;
    use std::sync::atomic::{AtomicU64, Ordering};

    static PROBE_ID: AtomicU64 = AtomicU64::new(1);
    let root = fallback_registry_root()?;
    if prepare_private_directory(&root).is_err() {
        return None;
    }
    let probe = root.join(format!(
        ".probe-{}-{}",
        std::process::id(),
        PROBE_ID.fetch_add(1, Ordering::Relaxed)
    ));
    let usable = match OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .open(&probe)
    {
        Ok(file) => {
            drop(file);
            remove_if_exists(&probe).is_ok()
        }
        Err(_) => false,
    };
    usable.then_some(root)
}

pub(super) fn process_is_alive(pid: u32) -> Result<bool> {
    ensure!(
        pid > 0 && pid <= libc::pid_t::MAX as u32,
        "workspace host metadata has an invalid PID"
    );
    let result = unsafe { libc::kill(pid as libc::pid_t, 0) };
    if result == 0 {
        return Ok(true);
    }
    let error = io::Error::last_os_error();
    match error.raw_os_error() {
        Some(libc::ESRCH) => Ok(false),
        Some(libc::EPERM) => Ok(true),
        _ => Err(error).context("cannot verify registered workspace host process"),
    }
}

/// Asks a host process to exit. Only hosts this build cannot speak to are ever
/// stopped this way; a compatible one answers a `Shutdown` request instead, so
/// its own refusal on unsaved buffers is what decides whether it stops.
pub(super) fn request_process_exit(pid: u32) -> Result<()> {
    ensure!(
        pid > 0 && pid <= libc::pid_t::MAX as u32,
        "workspace host metadata has an invalid PID"
    );
    if unsafe { libc::kill(pid as libc::pid_t, libc::SIGTERM) } == 0 {
        return Ok(());
    }
    let error = io::Error::last_os_error();
    match error.raw_os_error() {
        // Already gone is the outcome the caller wanted.
        Some(libc::ESRCH) => Ok(()),
        _ => Err(error).context("cannot stop the workspace host process"),
    }
}

fn is_stale_endpoint_error(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        cause.downcast_ref::<io::Error>().is_some_and(|error| {
            matches!(
                error.kind(),
                io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused
            )
        })
    })
}

#[derive(Debug)]
pub enum ServerEvent {
    Connected {
        id: u64,
        geometry: FrameGeometry,
        interactive: bool,
        /// Whether this client can hand a `:quit-here` directory to its shell.
        directory_handoff: bool,
        responses: ResponseSender,
    },
    Request {
        id: u64,
        request: ClientRequest,
    },
    /// A decoded request that violates validation or role rules. This crosses
    /// the same FIFO event boundary as valid requests so its error response
    /// cannot overtake an earlier semantic response on an unnumbered stream.
    ProtocolError {
        id: u64,
        message: String,
    },
    /// The connection ended because its framing failed: a malformed or
    /// truncated message, or a write that could not be completed.
    ///
    /// Separate from [`ServerEvent::ProtocolError`], which is a decoded
    /// request the host answers. Nothing can be answered here — the stream is
    /// no longer readable — so this carries the reason to whatever records it
    /// and is always followed by [`ServerEvent::Disconnected`].
    TransportFailure {
        id: u64,
        message: String,
    },
    Disconnected {
        id: u64,
    },
}

/// Semantic responses retain FIFO order; complete frames and terminal damage
/// share one replaceable slot. A slow client therefore holds at most one
/// pending visual update while lifecycle/command replies remain explicit.
#[derive(Clone, Debug)]
pub struct ResponseSender {
    messages: mpsc::Sender<HostResponse>,
    frame: tokio::sync::watch::Sender<Option<VisualResponse>>,
    next_visual: Arc<AtomicU64>,
    delivered_visual: Arc<AtomicU64>,
}

pub struct ResponseReceiver {
    messages: mpsc::Receiver<HostResponse>,
    frame: tokio::sync::watch::Receiver<Option<VisualResponse>>,
    delivered_visual: Arc<AtomicU64>,
    in_flight_visual: Option<u64>,
}

#[derive(Clone, Debug)]
struct VisualResponse {
    sequence: u64,
    response: HostResponse,
}

pub fn response_channel() -> (ResponseSender, ResponseReceiver) {
    let (messages, message_rx) = mpsc::channel(RESPONSE_CAPACITY);
    let (frame, frame_rx) = tokio::sync::watch::channel(None);
    let next_visual = Arc::new(AtomicU64::new(0));
    let delivered_visual = Arc::new(AtomicU64::new(0));
    (
        ResponseSender {
            messages,
            frame,
            next_visual,
            delivered_visual: delivered_visual.clone(),
        },
        ResponseReceiver {
            messages: message_rx,
            frame: frame_rx,
            delivered_visual,
            in_flight_visual: None,
        },
    )
}

impl ResponseSender {
    /// Whether the replaceable visual slot contains a frame the connection
    /// task has not finished writing. New damage cannot use that unseen frame
    /// as a base; its replacement must be self-contained.
    pub fn visual_pending(&self) -> bool {
        self.frame
            .borrow()
            .as_ref()
            .is_some_and(|visual| visual.sequence != self.delivered_visual.load(Ordering::Acquire))
    }

    pub fn try_send(
        &self,
        response: HostResponse,
    ) -> Result<(), mpsc::error::TrySendError<HostResponse>> {
        if matches!(
            response,
            HostResponse::Frame { .. } | HostResponse::TerminalDamage { .. }
        ) {
            let visual = VisualResponse {
                sequence: self
                    .next_visual
                    .fetch_add(1, Ordering::Relaxed)
                    .wrapping_add(1),
                response,
            };
            return self.frame.send(Some(visual)).map_err(|error| {
                mpsc::error::TrySendError::Closed(
                    error.0.expect("visual response is present").response,
                )
            });
        }
        self.messages.try_send(response)
    }

    pub async fn send(
        &self,
        response: HostResponse,
    ) -> Result<(), mpsc::error::SendError<HostResponse>> {
        if matches!(
            response,
            HostResponse::Frame { .. } | HostResponse::TerminalDamage { .. }
        ) {
            let visual = VisualResponse {
                sequence: self
                    .next_visual
                    .fetch_add(1, Ordering::Relaxed)
                    .wrapping_add(1),
                response,
            };
            return self.frame.send(Some(visual)).map_err(|error| {
                mpsc::error::SendError(error.0.expect("visual response is present").response)
            });
        }
        self.messages.send(response).await
    }
}

impl ResponseReceiver {
    async fn recv(&mut self) -> Option<HostResponse> {
        self.in_flight_visual = None;
        tokio::select! {
            biased;
            response = self.messages.recv() => response,
            changed = self.frame.changed() => {
                changed.ok()?;
                let visual = self.frame.borrow_and_update().clone()?;
                self.in_flight_visual = Some(visual.sequence);
                Some(visual.response)
            }
        }
    }

    fn mark_delivered(&mut self) {
        if let Some(sequence) = self.in_flight_visual.take() {
            self.delivered_visual.store(sequence, Ordering::Release);
        }
    }
}

pub struct LocalServer {
    events: mpsc::Receiver<ServerEvent>,
    task: tokio::task::JoinHandle<()>,
    shutdown: Option<oneshot::Sender<()>>,
    endpoint: LocalEndpoint,
    cleanup_on_drop: bool,
}

impl LocalServer {
    pub async fn bind(endpoint: &LocalEndpoint) -> Result<Self> {
        let listener = endpoint.bind().await?;
        let project_root_bytes = encode_path(&endpoint.project_root);
        let (events_tx, events) = mpsc::channel(EVENT_CAPACITY);
        let (shutdown, mut shutdown_rx) = oneshot::channel();
        let task = tokio::spawn(async move {
            let mut next_id = 1_u64;
            let connections = Arc::new(Semaphore::new(MAX_CONNECTIONS));
            loop {
                let accepted = tokio::select! {
                    biased;
                    _ = &mut shutdown_rx => {
                        use std::os::fd::AsRawFd;

                        // Closing a listener with a connection queued in the
                        // kernel can leave that socket connectable briefly on
                        // some platforms. Shut the socket down while this task
                        // still owns its descriptor, then let normal task exit
                        // drop and deregister it.
                        let _ = unsafe {
                            libc::shutdown(listener.as_raw_fd(), libc::SHUT_RDWR)
                        };
                        break;
                    },
                    accepted = listener.accept() => accepted,
                };
                let Ok((stream, _)) = accepted else {
                    break;
                };
                let Ok(permit) = connections.clone().try_acquire_owned() else {
                    continue;
                };
                let id = next_id;
                next_id = next_id.wrapping_add(1);
                let events = events_tx.clone();
                let project_root_bytes = project_root_bytes.clone();
                tokio::spawn(async move {
                    let _permit = permit;
                    let _ = serve_connection(id, stream, events, project_root_bytes).await;
                });
            }
        });
        Ok(Self {
            events,
            task,
            shutdown: Some(shutdown),
            endpoint: endpoint.clone(),
            cleanup_on_drop: true,
        })
    }

    pub async fn recv(&mut self) -> Option<ServerEvent> {
        self.events.recv().await
    }

    #[cfg(test)]
    async fn stop_abruptly(mut self) {
        self.cleanup_on_drop = false;
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        let _ = (&mut self.task).await;
    }
}

impl Drop for LocalServer {
    fn drop(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        self.task.abort();
        if self.cleanup_on_drop {
            // Stopping the accept task is asynchronous. Clean the published
            // endpoint synchronously so callers cannot observe this server as
            // live after its owner has been dropped.
            let _ = self.endpoint.cleanup();
        }
    }
}

pub struct LocalClient {
    reader: MessageReader<OwnedReadHalf>,
    writer: Option<OwnedWriteHalf>,
}

impl LocalClient {
    pub async fn connect(
        endpoint: &LocalEndpoint,
        geometry: FrameGeometry,
        interactive: bool,
    ) -> Result<Self> {
        Self::connect_with_handoff(endpoint, geometry, interactive, false).await
    }

    /// Connects and declares whether this client can hand a directory chosen by
    /// `:quit-here` to its shell.
    pub async fn connect_with_handoff(
        endpoint: &LocalEndpoint,
        geometry: FrameGeometry,
        interactive: bool,
        directory_handoff: bool,
    ) -> Result<Self> {
        let metadata = endpoint.verify_compatible_for_connect()?;
        let socket = decode_path(metadata.socket_bytes);
        let stream = UnixStream::connect(&socket)
            .await
            .with_context(|| format!("cannot attach to workspace host {}", socket.display()))?;
        let (reader, writer) = stream.into_split();
        let mut client = Self {
            reader: MessageReader::new(reader),
            writer: Some(writer),
        };
        client
            .send(&ClientRequest::Hello {
                protocol: PROTOCOL_VERSION,
                directory_handoff,
                features: if interactive {
                    vec![
                        FeatureGroup::Snapshots,
                        FeatureGroup::Input,
                        FeatureGroup::Buffers,
                        FeatureGroup::Wait,
                    ]
                } else {
                    vec![
                        FeatureGroup::Control,
                        FeatureGroup::Buffers,
                        FeatureGroup::Wait,
                    ]
                },
                project_root_bytes: encode_path(&endpoint.project_root),
                client_kind: if interactive {
                    ClientKind::Tui
                } else {
                    ClientKind::Control
                },
                client_version: CLIENT_VERSION.to_owned(),
                role: if interactive {
                    ClientRole::Interactive
                } else {
                    ClientRole::Control
                },
                geometry: geometry.into(),
            })
            .await?;
        Ok(client)
    }

    pub async fn send(&mut self, request: &ClientRequest) -> Result<()> {
        write_client_message(&mut self.writer, request).await
    }

    /// Cancellation-safe: a partially received response is retained by the
    /// reader, so racing this against terminal input in `select!` cannot
    /// desynchronize the stream.
    pub async fn recv(&mut self) -> Result<Option<HostResponse>> {
        self.reader.read().await
    }
}

async fn serve_connection<S>(
    id: u64,
    stream: S,
    events: mpsc::Sender<ServerEvent>,
    expected_project_root: Vec<u8>,
) -> Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let (reader, writer) = tokio::io::split(stream);
    let mut reader = MessageReader::new(reader);
    let hello = tokio::time::timeout(Duration::from_secs(2), reader.read::<ClientRequest>())
        .await
        .context("workspace client handshake timed out")??;
    let Some(hello) = hello else {
        bail!("workspace client disconnected before its handshake")
    };
    if let Err(message) = hello.validate() {
        let mut writer = writer;
        write_message(
            &mut writer,
            &HostResponse::Refused {
                message: format!("invalid client handshake: {message}"),
            },
        )
        .await?;
        return Ok(());
    }
    let ClientRequest::Hello {
        protocol,
        features,
        project_root_bytes,
        client_kind,
        client_version,
        role,
        geometry,
        directory_handoff,
    } = hello
    else {
        bail!("workspace client did not begin with a handshake")
    };
    let (responses, mut response_rx) = response_channel();
    if protocol != PROTOCOL_VERSION {
        let mut writer = writer;
        write_message(
            &mut writer,
            &HostResponse::Refused {
                message: format!(
                    "client protocol {protocol} is incompatible with host protocol {PROTOCOL_VERSION}"
                ),
            },
        )
        .await?;
        return Ok(());
    }
    if project_root_bytes != expected_project_root {
        let mut writer = writer;
        write_message(
            &mut writer,
            &HostResponse::Refused {
                message: "client requested a different workspace".to_owned(),
            },
        )
        .await?;
        return Ok(());
    }
    let interactive = role == ClientRole::Interactive;
    let identity_matches_role = matches!(
        (client_kind, role),
        (ClientKind::Tui, ClientRole::Interactive) | (ClientKind::Control, ClientRole::Control)
    );
    let expected_features: &[FeatureGroup] = if interactive {
        &[
            FeatureGroup::Snapshots,
            FeatureGroup::Input,
            FeatureGroup::Buffers,
            FeatureGroup::Wait,
        ]
    } else {
        &[
            FeatureGroup::Control,
            FeatureGroup::Buffers,
            FeatureGroup::Wait,
        ]
    };
    // The client must support everything its role needs, but may advertise
    // more in any order: a later bundled client can gain a feature group
    // without the host having to refuse it outright. Compatibility itself is
    // still gated by `PROTOCOL_VERSION` above.
    let features_cover_role = expected_features
        .iter()
        .all(|expected| features.contains(expected));
    if !identity_matches_role
        || client_version.len() > 128
        || features.len() > MAX_FEATURE_GROUPS
        || !features_cover_role
    {
        let mut writer = writer;
        write_message(
            &mut writer,
            &HostResponse::Refused {
                message: "client handshake identity or feature set is invalid".to_owned(),
            },
        )
        .await?;
        return Ok(());
    }
    events
        .send(ServerEvent::Connected {
            id,
            geometry: geometry.into(),
            interactive,
            directory_handoff,
            responses,
        })
        .await
        .context("workspace host stopped")?;
    let mut writer = writer;
    let result: Result<()> = async {
        loop {
            tokio::select! {
                // Semantic replies are bounded and must drain before another
                // ready request can advance teardown. In particular, a wait
                // client's periodic status poll can otherwise win this race
                // after the host has queued WaitState and ShuttingDown, then
                // observe the socket closing before either reply is written.
                // Visual responses already occupy one replaceable slot, so
                // this priority cannot starve reads indefinitely.
                biased;
                response = response_rx.recv() => {
                    let Some(response) = response else { break };
                    write_message(&mut writer, &response).await?;
                    response_rx.mark_delivered();
                }
                request = reader.read::<ClientRequest>() => {
                    match request? {
                        Some(ClientRequest::Hello { .. }) => {
                            if events.send(ServerEvent::ProtocolError {
                                id,
                                message: "handshake is only valid once".to_owned(),
                            }).await.is_err() {
                                break;
                            }
                        }
                        Some(request) => {
                            let protocol_error = request
                                .validate()
                                .err()
                                .or_else(|| (!request_allowed_for_role(&request, role)).then(|| {
                                    "request is not valid for this connection role".to_owned()
                                }));
                            let event = protocol_error.map_or(
                                ServerEvent::Request { id, request },
                                |message| ServerEvent::ProtocolError { id, message },
                            );
                            if events.send(event).await.is_err() {
                                break;
                            }
                        }
                        None => break,
                    }
                }
            }
        }
        Ok(())
    }
    .await;
    // The reason the loop ended is discarded by the caller that spawned this
    // task, so it is reported here or nowhere. A host would otherwise see a
    // truncated frame as an ordinary disconnection.
    if let Err(error) = &result {
        let _ = events
            .send(ServerEvent::TransportFailure {
                id,
                message: error.to_string(),
            })
            .await;
    }
    let _ = events.send(ServerEvent::Disconnected { id }).await;
    result
}

fn request_allowed_for_role(request: &ClientRequest, role: ClientRole) -> bool {
    match request {
        ClientRequest::Hello { .. } => false,
        ClientRequest::Input { .. }
        | ClientRequest::Invoke { .. }
        | ClientRequest::Notify { .. }
        | ClientRequest::AttachWait { .. }
        | ClientRequest::Pointer { .. }
        | ClientRequest::Resize { .. }
        | ClientRequest::Resynchronize
        | ClientRequest::Detach => role == ClientRole::Interactive,
        ClientRequest::RenameHost { .. } => role == ClientRole::Control,
        ClientRequest::Health
        | ClientRequest::SessionPreview
        | ClientRequest::ListBuffers
        | ClientRequest::ReadBuffer { .. }
        | ClientRequest::OpenBuffers { .. }
        | ClientRequest::ApplyTransaction { .. }
        | ClientRequest::SaveBuffer { .. }
        | ClientRequest::CloseBuffer { .. }
        | ClientRequest::CreateWait { .. }
        | ClientRequest::WaitStatus { .. }
        | ClientRequest::CompleteWaitBuffer { .. }
        | ClientRequest::CancelWait { .. }
        | ClientRequest::Shutdown
        | ClientRequest::ForceShutdown => true,
    }
}

/// A framed reader that owns the bytes of a partially received message.
///
/// Both transport loops read inside `tokio::select!`, which drops the losing
/// branch's future. Accumulating into a future-local buffer would discard
/// bytes already taken from the `BufReader` and resume mid-message, so the
/// partial message lives in the reader instead. `read` is therefore
/// cancellation-safe: its only await point is `fill_buf`, which is itself
/// cancellation-safe, and every byte consumed is already recorded in
/// `pending`.
struct MessageReader<R> {
    reader: BufReader<R>,
    pending: Vec<u8>,
}

impl<R: AsyncRead + Unpin> MessageReader<R> {
    fn new(reader: R) -> Self {
        Self {
            reader: BufReader::new(reader),
            pending: Vec::new(),
        }
    }

    async fn read<T: DeserializeOwned>(&mut self) -> Result<Option<T>> {
        loop {
            let available = self.reader.fill_buf().await?;
            if available.is_empty() {
                if self.pending.is_empty() {
                    return Ok(None);
                }
                let pending = self.pending.len();
                self.pending = Vec::new();
                bail!("workspace transport ended {pending} bytes inside a message");
            }
            let take = available
                .iter()
                .position(|byte| *byte == b'\n')
                .map_or(available.len(), |position| position + 1);
            if self.pending.len().saturating_add(take) > MAX_MESSAGE_BYTES {
                self.pending = Vec::new();
                bail!("workspace transport message exceeds {MAX_MESSAGE_BYTES} bytes");
            }
            self.pending.extend_from_slice(&available[..take]);
            self.reader.consume(take);
            if self.pending.last() == Some(&b'\n') {
                self.pending.pop();
                let message = serde_json::from_slice(&self.pending)
                    .context("malformed workspace transport message");
                self.pending = Vec::new();
                return message.map(Some);
            }
        }
    }
}

async fn write_message<W: AsyncWrite + Unpin, T: Serialize>(
    writer: &mut W,
    message: &T,
) -> Result<()> {
    write_message_with_timeout(writer, message, CONNECTION_WRITE_STALL).await
}

async fn write_client_message<W: AsyncWrite + Unpin, T: Serialize>(
    writer: &mut Option<W>,
    message: &T,
) -> Result<()> {
    let mut live = writer
        .take()
        .context("workspace transport writer is closed")?;
    match write_message(&mut live, message).await {
        Ok(()) => {
            *writer = Some(live);
            Ok(())
        }
        Err(error) => {
            let _ = live.shutdown().await;
            Err(error)
        }
    }
}

/// Frames one message, giving up only on a peer that accepts nothing.
///
/// Abandoning a write is safe before its first byte and never after: half a
/// frame on the stream leaves the peer to read a message that ends inside
/// itself, which is a transport error for what may only be a slow reader. A
/// local socket send buffer is small enough on some platforms that a whole
/// editor frame cannot fit in it, so a deadline spanning the message would
/// truncate every frame a client took a moment too long to drain. The budget
/// therefore covers a single write and restarts whenever the peer accepts any
/// byte at all, which still ends a connection that has genuinely stopped
/// reading.
async fn write_message_with_timeout<W: AsyncWrite + Unpin, T: Serialize>(
    writer: &mut W,
    message: &T,
    stall: Duration,
) -> Result<()> {
    let mut bytes = serde_json::to_vec(message)?;
    ensure!(
        bytes.len() < MAX_MESSAGE_BYTES,
        "workspace transport message exceeds {MAX_MESSAGE_BYTES} bytes"
    );
    bytes.push(b'\n');
    let mut written = 0;
    while written < bytes.len() {
        let count = tokio::time::timeout(stall, writer.write(&bytes[written..]))
            .await
            .context("workspace transport write timed out")??;
        ensure!(
            count > 0,
            "workspace transport peer stopped accepting bytes"
        );
        written += count;
    }
    tokio::time::timeout(stall, writer.flush())
        .await
        .context("workspace transport write timed out")??;
    Ok(())
}

fn write_json_atomic(path: &Path, value: &impl Serialize) -> Result<()> {
    use std::os::unix::fs::OpenOptionsExt;
    let temporary = path.with_extension(format!("tmp-{}", std::process::id()));
    let bytes = serde_json::to_vec_pretty(value)?;
    ensure!(
        bytes.len() <= MAX_METADATA_BYTES,
        "host metadata exceeds {MAX_METADATA_BYTES} bytes"
    );
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .open(&temporary)
        .with_context(|| format!("cannot create host metadata {}", temporary.display()))?;
    use std::io::Write;
    let result = (|| -> Result<()> {
        file.write_all(&bytes)?;
        file.sync_all()?;
        fs::rename(&temporary, path)?;
        if let Some(parent) = path.parent() {
            fs::File::open(parent)?.sync_all()?;
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = remove_if_exists(&temporary);
    }
    result
}

fn prepare_private_directory(path: &Path) -> Result<()> {
    if path.exists() {
        return verify_private(path, true);
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("cannot create directory parent {}", parent.display()))?;
    }
    match create_private_directory(path) {
        Ok(()) => Ok(()),
        Err(_) if path.exists() => verify_private(path, true),
        Err(error) => Err(error),
    }
}

fn verify_private(path: &Path, directory: bool) -> Result<()> {
    use std::os::unix::fs::MetadataExt;
    let metadata = fs::symlink_metadata(path)
        .with_context(|| format!("workspace host endpoint is missing: {}", path.display()))?;
    ensure!(
        !metadata.file_type().is_symlink(),
        "refusing symlinked host endpoint {}",
        path.display()
    );
    ensure!(
        metadata.uid() == unsafe { libc::geteuid() },
        "workspace host endpoint is owned by another user: {}",
        path.display()
    );
    ensure!(
        metadata.mode() & 0o077 == 0,
        "workspace host endpoint permissions are not private: {}",
        path.display()
    );
    ensure!(
        !directory || metadata.is_dir(),
        "workspace host directory is not a directory"
    );
    Ok(())
}

fn valid_runtime_directory(path: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    let Ok(metadata) = fs::symlink_metadata(path) else {
        return false;
    };
    path.is_absolute()
        && !metadata.file_type().is_symlink()
        && metadata.is_dir()
        && metadata.uid() == unsafe { libc::geteuid() }
        && metadata.mode() & 0o077 == 0
}

fn prepare_runtime_root(path: &Path) -> bool {
    use std::os::unix::fs::OpenOptionsExt;
    use std::sync::atomic::{AtomicU64, Ordering};

    static PROBE_ID: AtomicU64 = AtomicU64::new(1);
    if !valid_runtime_directory(path) {
        return false;
    }
    let runyte = path.join("runyte");
    // Two Runyte processes starting at once both find this directory missing
    // and both try to create it. Testing existence separately would make the
    // loser fall back to the workspace-root endpoint, so that the two could
    // no longer find each other. Create optimistically, then verify whoever
    // won left a private directory we own.
    if create_private_directory(&runyte).is_err() && !runyte.exists() {
        return false;
    }
    if verify_private(&runyte, true).is_err() {
        return false;
    }
    let probe = runyte.join(format!(
        ".probe-{}-{}",
        std::process::id(),
        PROBE_ID.fetch_add(1, Ordering::Relaxed)
    ));
    match OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .open(&probe)
    {
        Ok(file) => {
            drop(file);
            remove_if_exists(&probe).is_ok()
        }
        Err(_) => false,
    }
}

/// Restores the process umask when it leaves scope, including on unwind.
///
/// The umask is process-global, so this is deliberately held across one
/// `bind` and nothing else. It masks only group and other, leaving owner
/// permissions alone: anything another thread creates inside that window
/// stays usable but becomes private, never more accessible.
struct UmaskGuard(libc::mode_t);

impl UmaskGuard {
    fn narrow() -> Self {
        Self(unsafe { libc::umask(0o077) })
    }
}

impl Drop for UmaskGuard {
    fn drop(&mut self) {
        unsafe { libc::umask(self.0) };
    }
}

fn set_mode(path: &Path, mode: u32) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(mode))
        .with_context(|| format!("cannot secure host endpoint {}", path.display()))
}

fn create_private_directory(path: &Path) -> Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    let mut builder = fs::DirBuilder::new();
    builder.mode(0o700);
    builder
        .create(path)
        .with_context(|| format!("cannot create host directory {}", path.display()))
}

fn remove_if_exists(path: &Path) -> Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => {
            Err(error).with_context(|| format!("cannot remove stale endpoint {}", path.display()))
        }
    }
}

/// Retires the per-host directory after its socket and metadata are gone.
/// Shared registry and runtime roots remain; a concurrent file causes the
/// ordinary non-empty result and is never removed.
fn remove_empty_directory(path: &Path) -> Result<()> {
    match fs::remove_dir(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error)
            if matches!(
                error.raw_os_error(),
                Some(libc::ENOTEMPTY) | Some(libc::EEXIST)
            ) =>
        {
            Ok(())
        }
        Err(error) => Err(error)
            .with_context(|| format!("cannot remove retired host directory {}", path.display())),
    }
}

fn is_conclusive_stale(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::ConnectionRefused | io::ErrorKind::NotFound
    )
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        os::unix::fs::{MetadataExt, PermissionsExt},
        sync::atomic::{AtomicU64, Ordering},
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::*;
    use crate::{app::App, config::Config, workspace::WorkspaceHost};
    use tokio::io::AsyncReadExt;

    fn temporary_root() -> PathBuf {
        static NEXT_ROOT: AtomicU64 = AtomicU64::new(0);
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
            % 1_000_000_007;
        let sequence = NEXT_ROOT.fetch_add(1, Ordering::Relaxed);
        // macOS's ordinary per-user temp directory is already close to the
        // Unix-socket path limit. `/tmp` is available on supported Unix
        // platforms; canonicalizing it also avoids `/tmp` versus
        // `/private/tmp` identity differences on macOS.
        let base = Path::new("/tmp")
            .canonicalize()
            .unwrap_or_else(|_| std::env::temp_dir());
        base.join(format!(
            "ryt-{}-{unique:x}-{sequence:x}",
            std::process::id()
        ))
    }

    fn endpoint(_name: &str) -> (PathBuf, LocalEndpoint) {
        let root = temporary_root();
        fs::create_dir_all(&root).unwrap();
        let endpoint = LocalEndpoint::new(&root.join(".runyte"), &root).unwrap();
        (root, endpoint)
    }

    #[test]
    fn constructing_endpoint_is_side_effect_free() {
        let root = temporary_root();
        let workspace = root.join(".runyte");
        let _endpoint = LocalEndpoint::new(&workspace, &root).unwrap();
        assert!(!workspace.exists());
    }

    #[test]
    fn registry_entry_count_is_bounded_while_the_directory_is_streamed() {
        let root = temporary_root();
        let registry = root.join("registry");
        prepare_private_directory(&registry).unwrap();
        fs::write(registry.join("ignored.txt"), b"not a registry row").unwrap();
        for index in 0..=MAX_REGISTERED_HOSTS {
            fs::write(registry.join(format!("{index:04}.json")), b"{}").unwrap();
        }

        let error = registered_hosts_in(std::slice::from_ref(&registry)).unwrap_err();
        assert!(error.to_string().contains("more than"), "{error:#}");

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn endpoint_path_limit_follows_the_platform_socket_capacity() {
        const SUFFIX: &str = "/host/workspace.sock";

        let state_root_for_socket_length = |length: usize| {
            let component_length = length.checked_sub(1 + SUFFIX.len()).unwrap();
            PathBuf::from(format!("/{}", "x".repeat(component_length)))
        };
        let capacity = socket_path_capacity();
        let allowed = state_root_for_socket_length(capacity);
        let oversized = state_root_for_socket_length(capacity + 1);

        assert!(LocalEndpoint::new(&allowed, Path::new("/project")).is_ok());
        let error = LocalEndpoint::new(&oversized, Path::new("/project")).unwrap_err();
        assert!(error.to_string().contains("socket path is too long"));
    }

    #[test]
    fn a_shared_secondary_registry_serializes_different_primary_roots() {
        let root = temporary_root();
        let first = root.join("cache-a");
        let second = root.join("cache-b");
        let shared = root.join("runtime");
        let (first_ready_tx, first_ready_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let (second_ready_tx, second_ready_rx) = std::sync::mpsc::channel();

        std::thread::scope(|scope| {
            let first_primary = first.clone();
            let first_shared = shared.clone();
            scope.spawn(move || {
                let _locks = PrivateFileLock::acquire(
                    Some(&first_primary),
                    Some(&first_shared),
                    None,
                    ".test.lock",
                    "test",
                )
                .unwrap();
                first_ready_tx.send(()).unwrap();
                release_rx.recv().unwrap();
            });
            first_ready_rx.recv().unwrap();
            let second_primary = second.clone();
            let second_shared = shared.clone();
            scope.spawn(move || {
                let _locks = PrivateFileLock::acquire(
                    Some(&second_primary),
                    Some(&second_shared),
                    None,
                    ".test.lock",
                    "test",
                )
                .unwrap();
                second_ready_tx.send(()).unwrap();
            });
            assert!(matches!(
                second_ready_rx.recv_timeout(Duration::from_millis(100)),
                Err(std::sync::mpsc::RecvTimeoutError::Timeout)
            ));
            release_tx.send(()).unwrap();
            second_ready_rx
                .recv_timeout(Duration::from_secs(1))
                .unwrap();
        });

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn symlinked_host_directory_is_refused_without_changing_target_permissions() {
        use std::os::unix::fs::symlink;

        let (root, endpoint) = endpoint("symlink-directory");
        let host = endpoint.metadata().parent().unwrap();
        fs::create_dir_all(host.parent().unwrap()).unwrap();
        let target = root.join("target");
        fs::create_dir(&target).unwrap();
        fs::set_permissions(&target, fs::Permissions::from_mode(0o755)).unwrap();
        symlink(&target, host).unwrap();

        let error = endpoint.prepare_directory().unwrap_err().to_string();
        assert!(error.contains("symlinked host endpoint"), "{error}");
        assert_eq!(fs::metadata(&target).unwrap().mode() & 0o777, 0o755);
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn symlinked_owner_wide_inventory_is_refused_without_following_it() {
        use std::os::unix::fs::symlink;

        let (root, fallback) = endpoint("symlink-inventory");
        let runtime = temporary_root();
        fs::create_dir(&runtime).unwrap();
        fs::set_permissions(&runtime, fs::Permissions::from_mode(0o700)).unwrap();
        let endpoint = LocalEndpoint::discover_with_runtime(
            fallback.directory.parent().unwrap(),
            &root,
            Some(&runtime),
        )
        .unwrap();
        let target = root.join("inventory-target");
        fs::create_dir(&target).unwrap();
        fs::set_permissions(&target, fs::Permissions::from_mode(0o755)).unwrap();
        symlink(&target, runtime.join("runyte/all-hosts")).unwrap();

        let error = match LocalServer::bind(&endpoint).await {
            Ok(_) => panic!("symlinked owner-wide inventory was accepted"),
            Err(error) => error.to_string(),
        };
        assert!(error.contains("symlinked host endpoint"), "{error}");
        assert_eq!(fs::metadata(&target).unwrap().mode() & 0o777, 0o755);

        fs::remove_dir_all(runtime).unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn discovery_prefers_only_a_private_user_runtime_directory() {
        let (root, fallback) = endpoint("runtime-discovery");
        let runtime = temporary_root();
        fs::create_dir(&runtime).unwrap();
        fs::set_permissions(&runtime, fs::Permissions::from_mode(0o700)).unwrap();
        let preferred = LocalEndpoint::discover_with_runtime(
            fallback.directory.parent().unwrap(),
            &root,
            Some(&runtime),
        )
        .unwrap();
        assert!(preferred.socket().starts_with(runtime.join("runyte")));

        fs::set_permissions(&runtime, fs::Permissions::from_mode(0o755)).unwrap();
        let rejected = LocalEndpoint::discover_with_runtime(
            fallback.directory.parent().unwrap(),
            &root,
            Some(&runtime),
        )
        .unwrap();
        assert_eq!(rejected.socket(), fallback.socket());
        fs::remove_dir_all(runtime).unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn owner_wide_inventory_is_durable_and_boot_scoped() {
        let home = Path::new("/accounts/example");
        let inventory = all_hosts_registry_root_for_home(home, "boot-one");
        assert!(inventory.starts_with(home));
        assert_eq!(inventory.file_name().unwrap(), "boot-one");
        assert_ne!(
            inventory,
            all_hosts_registry_root_for_home(home, "boot-two")
        );
        assert!(!inventory.starts_with(std::env::temp_dir()));
        #[cfg(target_os = "macos")]
        assert_eq!(
            inventory,
            home.join("Library/Application Support/Runyte/all-hosts/boot-one")
        );
        #[cfg(not(target_os = "macos"))]
        assert_eq!(
            inventory,
            home.join(".local/state/runyte/all-hosts/boot-one")
        );
    }

    #[test]
    fn boot_namespace_is_stable_and_path_safe() {
        let first = boot_namespace().unwrap();
        let second = boot_namespace().unwrap();
        assert_eq!(first, second);
        assert_eq!(first.len(), HOST_ID_LENGTH);
        assert!(first.bytes().all(|byte| byte.is_ascii_hexdigit()));
    }

    async fn bind_or_skip(endpoint: &LocalEndpoint) -> Option<LocalServer> {
        match LocalServer::bind(endpoint).await {
            Ok(server) => Some(server),
            Err(error)
                if error.chain().any(|cause| {
                    cause
                        .downcast_ref::<io::Error>()
                        .is_some_and(|error| error.raw_os_error() == Some(libc::EPERM))
                }) =>
            {
                None
            }
            Err(error) => panic!("cannot bind test transport: {error:#}"),
        }
    }

    /// Rewrites the published metadata, keeping everything but the protocol
    /// and the process it records. This is what an endpoint left by a host of
    /// another version looks like: its shape predates registered identities,
    /// so it carries no `id` either.
    fn republish_metadata(endpoint: &LocalEndpoint, protocol: u32, pid: u32) {
        let metadata = serde_json::json!({
            "protocol": protocol,
            "pid": pid,
            "project_root_bytes": encode_path(&endpoint.project_root),
            "socket_bytes": encode_path(endpoint.socket()),
        });
        fs::write(endpoint.metadata(), serde_json::to_vec(&metadata).unwrap()).unwrap();
        set_mode(endpoint.metadata(), 0o600).unwrap();
    }

    /// A PID that has certainly been released: the process ran to completion
    /// and was reaped, so nothing answers for it.
    fn exited_process_id() -> u32 {
        let mut child = std::process::Command::new("/bin/sh")
            .arg("-c")
            .arg("exit 0")
            .spawn()
            .unwrap();
        let pid = child.id();
        child.wait().unwrap();
        pid
    }

    #[tokio::test]
    async fn an_incompatible_endpoint_is_stale_once_its_host_has_exited() {
        let (root, endpoint) = endpoint("incompatible-liveness");
        let Some(_server) = bind_or_skip(&endpoint).await else {
            fs::remove_dir_all(root).unwrap();
            return;
        };
        let older = PROTOCOL_VERSION.checked_sub(1).unwrap();

        republish_metadata(&endpoint, older, std::process::id());
        let error = endpoint.verify_compatible_for_connect().unwrap_err();
        let incompatible = error.downcast_ref::<IncompatibleHost>().unwrap_or_else(|| {
            panic!("a running host of another protocol was not named: {error:#}")
        });
        assert_eq!(incompatible.protocol, older);
        assert_eq!(incompatible.pid, std::process::id());
        assert!(!is_stale_endpoint_error(&error));
        // A host that is still running owns its endpoint, so discovery has to
        // be able to describe it rather than pretend the workspace is idle.
        let published = endpoint.published_host().unwrap().unwrap();
        assert_eq!(published.protocol, older);
        assert!(!published.speaks_current_protocol());

        // The same files, once nothing is running behind them, are leftovers.
        // Refusing them on the protocol would make them permanent: no caller
        // would ever be allowed to replace the endpoint.
        republish_metadata(&endpoint, older, exited_process_id());
        let error = endpoint.verify_compatible_for_connect().unwrap_err();
        assert!(
            error.downcast_ref::<IncompatibleHost>().is_none(),
            "{error:#}"
        );
        assert!(is_stale_endpoint_error(&error), "{error:#}");

        endpoint.cleanup().unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn endpoint_metadata_is_atomic_private_and_stale_socket_recovers() {
        let (root, endpoint) = endpoint("metadata");
        let Some(server) = bind_or_skip(&endpoint).await else {
            fs::remove_dir_all(root).unwrap();
            return;
        };
        let metadata = endpoint.verify_for_connect().unwrap();
        assert_eq!(metadata.protocol, PROTOCOL_VERSION);
        assert_eq!(
            fs::metadata(endpoint.metadata()).unwrap().mode() & 0o777,
            0o600
        );
        assert_eq!(
            fs::metadata(endpoint.socket()).unwrap().mode() & 0o777,
            0o600
        );
        server.stop_abruptly().await;
        let mut metadata = endpoint.verify_for_connect().unwrap();
        metadata.pid = 2_000_000_000;
        fs::write(
            endpoint.metadata(),
            serde_json::to_vec_pretty(&metadata).unwrap(),
        )
        .unwrap();

        let replacement = LocalServer::bind(&endpoint).await.unwrap();
        drop(replacement);
        endpoint.cleanup().unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn registry_lists_names_rejects_duplicates_and_preserves_a_name_across_restart() {
        let (first_root, first_fallback) = endpoint("registry-first");
        let (second_root, second_fallback) = endpoint("registry-second");
        let runtime = temporary_root();
        fs::create_dir(&runtime).unwrap();
        fs::set_permissions(&runtime, fs::Permissions::from_mode(0o700)).unwrap();
        let first = LocalEndpoint::discover_with_runtime(
            first_fallback.directory.parent().unwrap(),
            &first_root,
            Some(&runtime),
        )
        .unwrap();
        let second = LocalEndpoint::discover_with_runtime(
            second_fallback.directory.parent().unwrap(),
            &second_root,
            Some(&runtime),
        )
        .unwrap();
        let Some(first_server) = bind_or_skip(&first).await else {
            fs::remove_dir_all(runtime).unwrap();
            fs::remove_dir_all(first_root).unwrap();
            fs::remove_dir_all(second_root).unwrap();
            return;
        };
        let Some(second_server) = bind_or_skip(&second).await else {
            drop(first_server);
            first.cleanup().unwrap();
            fs::remove_dir_all(runtime).unwrap();
            fs::remove_dir_all(first_root).unwrap();
            fs::remove_dir_all(second_root).unwrap();
            return;
        };

        first.rename("backend").unwrap();
        let error = second.rename("backend").unwrap_err().to_string();
        assert!(error.contains("already used"), "{error}");
        let registry = runtime.join("runyte/hosts");
        let hosts = registered_hosts_in(std::slice::from_ref(&registry)).unwrap();
        assert_eq!(hosts.len(), 2);
        let inventory = runtime.join("runyte/all-hosts");
        let inventory_entries = fs::read_dir(&inventory)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect::<Vec<_>>();
        assert_eq!(inventory_entries.len(), 2);
        assert!(
            inventory_entries.iter().all(|path| path
                .extension()
                .is_some_and(|extension| extension == "json")),
            "owner-wide inventory must not retain identity lock files"
        );
        assert_eq!(
            registered_hosts_in(std::slice::from_ref(&inventory))
                .unwrap()
                .len(),
            2
        );
        assert_eq!(
            hosts
                .iter()
                .find(|host| host.id == first.id())
                .and_then(|host| host.name.as_deref()),
            Some("backend")
        );

        drop(first_server);
        first.cleanup().unwrap();
        let replacement = LocalServer::bind(&first).await.unwrap();
        assert_eq!(
            first.verify_for_connect().unwrap().name.as_deref(),
            Some("backend")
        );

        drop(replacement);
        drop(second_server);
        first.cleanup().unwrap();
        second.cleanup().unwrap();
        assert!(!first.directory.exists());
        assert!(!second.directory.exists());
        assert!(
            registered_hosts_in(std::slice::from_ref(&registry))
                .unwrap()
                .is_empty()
        );
        assert!(
            registered_hosts_in(std::slice::from_ref(&inventory))
                .unwrap()
                .is_empty()
        );
        fs::remove_dir_all(runtime).unwrap();
        fs::remove_dir_all(first_root).unwrap();
        fs::remove_dir_all(second_root).unwrap();
    }

    #[test]
    fn registry_scan_keeps_a_live_row_before_endpoint_readiness() {
        let (root, fallback) = endpoint("registry-publication-window");
        let runtime = temporary_root();
        fs::create_dir(&runtime).unwrap();
        fs::set_permissions(&runtime, fs::Permissions::from_mode(0o700)).unwrap();
        let endpoint = LocalEndpoint::discover_with_runtime(
            fallback.directory.parent().unwrap(),
            &root,
            Some(&runtime),
        )
        .unwrap();
        endpoint.prepare_directory().unwrap();

        let registry = runtime.join("runyte/hosts");
        prepare_private_directory(&registry).unwrap();
        let registration = registry.join(format!("{}.json", endpoint.id()));
        let metadata = EndpointMetadata {
            protocol: PROTOCOL_VERSION,
            pid: std::process::id(),
            id: endpoint.id().to_owned(),
            name: None,
            project_root_bytes: encode_path(&root),
            socket_bytes: encode_path(endpoint.socket()),
        };
        write_json_atomic(&registration, &metadata).unwrap();
        assert!(!endpoint.metadata().exists());

        assert!(
            registered_hosts_in(std::slice::from_ref(&registry))
                .unwrap()
                .is_empty()
        );
        assert!(
            registration.exists(),
            "a concurrent scan deleted the live host's early registry row"
        );

        fs::remove_dir_all(runtime).unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn malformed_registry_rows_do_not_hide_valid_hosts() {
        let (root, fallback) = endpoint("malformed-registry");
        let runtime = temporary_root();
        fs::create_dir(&runtime).unwrap();
        fs::set_permissions(&runtime, fs::Permissions::from_mode(0o700)).unwrap();
        let endpoint = LocalEndpoint::discover_with_runtime(
            fallback.directory.parent().unwrap(),
            &root,
            Some(&runtime),
        )
        .unwrap();
        let Some(server) = bind_or_skip(&endpoint).await else {
            fs::remove_dir_all(runtime).unwrap();
            fs::remove_dir_all(root).unwrap();
            return;
        };
        let registry = runtime.join("runyte/hosts");
        let malformed = registry.join("malformed.json");
        fs::write(&malformed, b"not json").unwrap();
        fs::set_permissions(&malformed, fs::Permissions::from_mode(0o600)).unwrap();
        let oversized = registry.join("oversized.json");
        fs::write(&oversized, vec![b'x'; 64 * 1024 + 1]).unwrap();
        fs::set_permissions(&oversized, fs::Permissions::from_mode(0o600)).unwrap();

        let hosts = registered_hosts_in(std::slice::from_ref(&registry)).unwrap();
        assert_eq!(hosts.len(), 1);
        assert_eq!(hosts[0].id, endpoint.id());

        drop(server);
        endpoint.cleanup().unwrap();
        fs::remove_dir_all(runtime).unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn concurrent_hosts_cannot_claim_the_same_name() {
        let (first_root, first_fallback) = endpoint("name-race-first");
        let (second_root, second_fallback) = endpoint("name-race-second");
        let runtime = temporary_root();
        fs::create_dir(&runtime).unwrap();
        fs::set_permissions(&runtime, fs::Permissions::from_mode(0o700)).unwrap();
        let first = LocalEndpoint::discover_with_runtime(
            first_fallback.directory.parent().unwrap(),
            &first_root,
            Some(&runtime),
        )
        .unwrap();
        let second = LocalEndpoint::discover_with_runtime(
            second_fallback.directory.parent().unwrap(),
            &second_root,
            Some(&runtime),
        )
        .unwrap();
        let Some(first_server) = bind_or_skip(&first).await else {
            fs::remove_dir_all(runtime).unwrap();
            fs::remove_dir_all(first_root).unwrap();
            fs::remove_dir_all(second_root).unwrap();
            return;
        };
        let Some(second_server) = bind_or_skip(&second).await else {
            drop(first_server);
            first.cleanup().unwrap();
            fs::remove_dir_all(runtime).unwrap();
            fs::remove_dir_all(first_root).unwrap();
            fs::remove_dir_all(second_root).unwrap();
            return;
        };

        let barrier = std::sync::Arc::new(std::sync::Barrier::new(3));
        let (first_result, second_result) = std::thread::scope(|scope| {
            let first_barrier = barrier.clone();
            let first_endpoint = first.clone();
            let first_thread = scope.spawn(move || {
                first_barrier.wait();
                first_endpoint.rename("shared")
            });
            let second_barrier = barrier.clone();
            let second_endpoint = second.clone();
            let second_thread = scope.spawn(move || {
                second_barrier.wait();
                second_endpoint.rename("shared")
            });
            barrier.wait();
            (first_thread.join().unwrap(), second_thread.join().unwrap())
        });
        assert_ne!(first_result.is_ok(), second_result.is_ok());
        let refusal = first_result
            .err()
            .or_else(|| second_result.err())
            .unwrap()
            .to_string();
        assert!(refusal.contains("already used"), "{refusal}");
        let registry = runtime.join("runyte/hosts");
        assert_eq!(
            registered_hosts_in(std::slice::from_ref(&registry))
                .unwrap()
                .iter()
                .filter(|host| host.name.as_deref() == Some("shared"))
                .count(),
            1
        );

        drop(first_server);
        drop(second_server);
        first.cleanup().unwrap();
        second.cleanup().unwrap();
        fs::remove_dir_all(runtime).unwrap();
        fs::remove_dir_all(first_root).unwrap();
        fs::remove_dir_all(second_root).unwrap();
    }

    #[tokio::test]
    async fn old_cleanup_cannot_remove_a_replacement_hosts_registration() {
        let (root, fallback) = endpoint("cleanup-race");
        let runtime = temporary_root();
        fs::create_dir(&runtime).unwrap();
        fs::set_permissions(&runtime, fs::Permissions::from_mode(0o700)).unwrap();
        let registry = runtime.join("registry");
        let workspace = fallback.directory.parent().unwrap();
        let first = LocalEndpoint::at_directory(
            runtime.join("first"),
            workspace,
            &root,
            EndpointPublication {
                registry: Some(registry.clone()),
                secondary_registry: None,
                inventory_registry: InventoryRegistry::Disabled,
                test_supervisor: None,
                runtime_root: None,
            },
        )
        .unwrap();
        let second = LocalEndpoint::at_directory(
            runtime.join("second"),
            workspace,
            &root,
            EndpointPublication {
                registry: Some(registry.clone()),
                secondary_registry: None,
                inventory_registry: InventoryRegistry::Disabled,
                test_supervisor: None,
                runtime_root: None,
            },
        )
        .unwrap();

        let Some(first_server) = bind_or_skip(&first).await else {
            fs::remove_dir_all(runtime).unwrap();
            fs::remove_dir_all(root).unwrap();
            return;
        };
        drop(first_server);
        let Some(second_server) = bind_or_skip(&second).await else {
            first.cleanup().unwrap();
            fs::remove_dir_all(runtime).unwrap();
            fs::remove_dir_all(root).unwrap();
            return;
        };
        first.cleanup().unwrap();

        assert!(second.verify_for_connect().is_ok());
        let registration = registry.join(format!("{}.json", second.id()));
        let metadata: EndpointMetadata =
            serde_json::from_slice(&fs::read(registration).unwrap()).unwrap();
        assert_eq!(decode_path(metadata.socket_bytes), second.socket());

        drop(second_server);
        second.cleanup().unwrap();
        fs::remove_dir_all(runtime).unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn dead_registry_entries_are_removed_while_listing() {
        let (root, fallback) = endpoint("dead-registry");
        let runtime = temporary_root();
        fs::create_dir(&runtime).unwrap();
        fs::set_permissions(&runtime, fs::Permissions::from_mode(0o700)).unwrap();
        let endpoint = LocalEndpoint::discover_with_runtime(
            fallback.directory.parent().unwrap(),
            &root,
            Some(&runtime),
        )
        .unwrap();
        let Some(server) = bind_or_skip(&endpoint).await else {
            fs::remove_dir_all(runtime).unwrap();
            fs::remove_dir_all(root).unwrap();
            return;
        };
        let record = runtime
            .join("runyte/hosts")
            .join(format!("{}.json", endpoint.id()));
        let mut metadata: EndpointMetadata =
            serde_json::from_slice(&fs::read(&record).unwrap()).unwrap();
        // Simulate a crashed host whose PID has already been reused by this
        // still-live test process. PID liveness alone must not retain it.
        metadata.pid = std::process::id();
        write_json_atomic(&record, &metadata).unwrap();
        server.stop_abruptly().await;

        assert!(
            registered_hosts_in(&[runtime.join("runyte/hosts")])
                .unwrap()
                .is_empty()
        );
        assert!(!record.exists());

        endpoint.cleanup().unwrap();
        fs::remove_dir_all(runtime).unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn an_unobservable_pid_does_not_remove_a_responsive_endpoint() {
        let (root, fallback) = endpoint("hidden-pid-registry");
        let runtime = temporary_root();
        fs::create_dir(&runtime).unwrap();
        fs::set_permissions(&runtime, fs::Permissions::from_mode(0o700)).unwrap();
        let endpoint = LocalEndpoint::discover_with_runtime(
            fallback.directory.parent().unwrap(),
            &root,
            Some(&runtime),
        )
        .unwrap();
        let Some(server) = bind_or_skip(&endpoint).await else {
            fs::remove_dir_all(runtime).unwrap();
            fs::remove_dir_all(root).unwrap();
            return;
        };
        let record = runtime
            .join("runyte/hosts")
            .join(format!("{}.json", endpoint.id()));
        let mut metadata: EndpointMetadata =
            serde_json::from_slice(&fs::read(endpoint.metadata()).unwrap()).unwrap();
        metadata.pid = 2_000_000_000;
        write_json_atomic(endpoint.metadata(), &metadata).unwrap();
        write_json_atomic(&record, &metadata).unwrap();

        let hosts = registered_hosts_in(&[runtime.join("runyte/hosts")]).unwrap();
        assert_eq!(hosts.len(), 1);
        assert!(record.exists());

        metadata.pid = std::process::id();
        write_json_atomic(endpoint.metadata(), &metadata).unwrap();
        write_json_atomic(&record, &metadata).unwrap();
        drop(server);
        endpoint.cleanup().unwrap();
        fs::remove_dir_all(runtime).unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn hostile_endpoint_permissions_are_refused() {
        let (root, endpoint) = endpoint("permissions");
        let Some(server) = bind_or_skip(&endpoint).await else {
            fs::remove_dir_all(root).unwrap();
            return;
        };
        let directory = endpoint.metadata().parent().unwrap();
        fs::set_permissions(directory, fs::Permissions::from_mode(0o755)).unwrap();
        let error = endpoint.verify_for_connect().unwrap_err().to_string();
        assert!(error.contains("permissions are not private"), "{error}");
        fs::set_permissions(directory, fs::Permissions::from_mode(0o700)).unwrap();
        assert_eq!(fs::metadata(directory).unwrap().uid(), unsafe {
            libc::geteuid()
        });
        drop(server);
        endpoint.cleanup().unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn endpoint_metadata_is_bounded_and_validated_before_acceptance() {
        let (root, endpoint) = endpoint("bounded-metadata");
        let Some(server) = bind_or_skip(&endpoint).await else {
            fs::remove_dir_all(root).unwrap();
            return;
        };
        let original = fs::read(endpoint.metadata()).unwrap();
        let mut metadata: EndpointMetadata = serde_json::from_slice(&original).unwrap();

        fs::write(endpoint.metadata(), vec![b' '; MAX_METADATA_BYTES + 1]).unwrap();
        let error = endpoint.verify_for_connect().unwrap_err().to_string();
        assert!(error.contains("exceeds"), "{error}");

        metadata.project_root_bytes = vec![b'/'; MAX_PERSISTED_PATH_BYTES + 1];
        fs::write(endpoint.metadata(), serde_json::to_vec(&metadata).unwrap()).unwrap();
        let error = format!("{:#}", endpoint.verify_for_connect().unwrap_err());
        assert!(error.contains("project directory exceeds"), "{error}");

        metadata.project_root_bytes = encode_path(&root);
        metadata.name = Some("x".repeat(MAX_HOST_NAME_BYTES + 1));
        fs::write(endpoint.metadata(), serde_json::to_vec(&metadata).unwrap()).unwrap();
        let error = format!("{:#}", endpoint.verify_for_connect().unwrap_err());
        assert!(error.contains("session name cannot exceed"), "{error}");

        fs::write(endpoint.metadata(), original).unwrap();
        drop(server);
        endpoint.cleanup().unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn stored_session_name_input_is_byte_bounded() {
        let (root, endpoint) = endpoint("bounded-stored-name");
        let name_file = endpoint.name_file.as_ref().unwrap();
        prepare_private_directory(name_file.parent().unwrap()).unwrap();
        fs::write(name_file, vec![b'x'; MAX_STORED_NAME_BYTES + 1]).unwrap();
        fs::set_permissions(name_file, fs::Permissions::from_mode(0o600)).unwrap();

        let error = endpoint.load_stored_name().unwrap_err().to_string();
        assert!(error.contains("exceeds"), "{error}");

        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn handshake_is_versioned_and_response_backpressure_is_bounded() {
        let (root, endpoint) = endpoint("handshake");
        let Some(mut server) = bind_or_skip(&endpoint).await else {
            fs::remove_dir_all(root).unwrap();
            return;
        };
        let geometry = FrameGeometry::default();
        let mut client = LocalClient::connect(&endpoint, geometry, true)
            .await
            .unwrap();
        let ServerEvent::Connected {
            interactive,
            responses,
            ..
        } = server.recv().await.unwrap()
        else {
            panic!("expected connection")
        };
        assert!(interactive);
        responses
            .send(HostResponse::Welcome {
                protocol: PROTOCOL_VERSION,
                pid: std::process::id(),
                features: vec![FeatureGroup::Snapshots, FeatureGroup::Input],
                host_version: CLIENT_VERSION.to_owned(),
            })
            .await
            .unwrap();
        assert!(matches!(
            client.recv().await.unwrap(),
            Some(HostResponse::Welcome { .. })
        ));
        let (responses, _held_receiver) = mpsc::channel(RESPONSE_CAPACITY);
        for index in 0..RESPONSE_CAPACITY {
            responses
                .try_send(HostResponse::Error {
                    message: index.to_string(),
                })
                .unwrap();
        }
        assert!(
            responses
                .try_send(HostResponse::Error {
                    message: "slow".to_owned(),
                })
                .is_err()
        );
        drop(server);
        endpoint.cleanup().unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn visual_slot_stays_pending_until_the_connection_finishes_delivery() {
        let mut host = WorkspaceHost::new(App::new(Config::default(), None).unwrap());
        let first = host.prepare_frame(FrameGeometry::default());
        let second = host.prepare_frame(FrameGeometry::default());
        let (responses, mut receiver) = response_channel();
        assert!(!responses.visual_pending());

        responses
            .try_send(HostResponse::Frame {
                frame: Box::new(first.into()),
            })
            .unwrap();
        responses
            .try_send(HostResponse::Frame {
                frame: Box::new(second.into()),
            })
            .unwrap();
        assert!(responses.visual_pending());

        let delivered = receiver.recv().await.unwrap();
        assert!(matches!(delivered, HostResponse::Frame { .. }));
        assert!(
            responses.visual_pending(),
            "taking a frame is not delivery while its socket write is pending"
        );
        receiver.mark_delivered();
        assert!(!responses.visual_pending());
    }

    #[tokio::test]
    async fn mismatched_handshake_is_actionably_refused() {
        let (root, endpoint) = endpoint("mismatch");
        let Some(_server) = bind_or_skip(&endpoint).await else {
            fs::remove_dir_all(root).unwrap();
            return;
        };
        let stream = UnixStream::connect(endpoint.socket()).await.unwrap();
        let (reader, mut writer) = stream.into_split();
        write_message(
            &mut writer,
            &ClientRequest::Hello {
                protocol: PROTOCOL_VERSION + 1,
                directory_handoff: false,
                features: vec![FeatureGroup::Snapshots, FeatureGroup::Input],
                project_root_bytes: encode_path(&endpoint.project_root),
                client_kind: ClientKind::Tui,
                client_version: CLIENT_VERSION.to_owned(),
                role: ClientRole::Interactive,
                geometry: FrameGeometry::default().into(),
            },
        )
        .await
        .unwrap();
        let mut reader = MessageReader::new(reader);
        let response: HostResponse = reader.read().await.unwrap().unwrap();
        assert!(matches!(
            response,
            HostResponse::Refused { message }
                if message.contains("incompatible") && message.contains("protocol")
        ));
        endpoint.cleanup().unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn invalid_handshake_fields_are_refused_before_connection() {
        let (root, endpoint) = endpoint("invalid-handshake");
        let Some(mut server) = bind_or_skip(&endpoint).await else {
            fs::remove_dir_all(root).unwrap();
            return;
        };
        let stream = UnixStream::connect(endpoint.socket()).await.unwrap();
        let (reader, mut writer) = stream.into_split();
        write_message(
            &mut writer,
            &ClientRequest::Hello {
                protocol: PROTOCOL_VERSION,
                directory_handoff: false,
                features: vec![
                    FeatureGroup::Control,
                    FeatureGroup::Buffers,
                    FeatureGroup::Wait,
                ],
                project_root_bytes: encode_path(&endpoint.project_root),
                client_kind: ClientKind::Control,
                client_version: String::new(),
                role: ClientRole::Control,
                geometry: FrameGeometry::default().into(),
            },
        )
        .await
        .unwrap();
        let response: HostResponse = MessageReader::new(reader).read().await.unwrap().unwrap();
        assert!(matches!(
            response,
            HostResponse::Refused { message } if message.contains("version length")
        ));
        assert!(
            tokio::time::timeout(Duration::from_millis(100), server.recv())
                .await
                .is_err(),
            "an invalid handshake reached the workspace host"
        );
        endpoint.cleanup().unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn malformed_established_messages_always_disconnect_host_state() {
        for (name, payload) in [
            ("malformed-established", b"{not-json}\n".as_slice()),
            ("truncated-established", b"{\"type\":\"health\"".as_slice()),
        ] {
            let (host_stream, stream) = tokio::io::duplex(MAX_MESSAGE_BYTES + 1);
            let (events, mut server) = mpsc::channel(EVENT_CAPACITY);
            let project_root = encode_path(Path::new("/malformed-frame-test"));
            tokio::spawn(serve_connection(
                17,
                host_stream,
                events,
                project_root.clone(),
            ));
            let (reader, mut writer) = tokio::io::split(stream);
            write_message(
                &mut writer,
                &ClientRequest::Hello {
                    protocol: PROTOCOL_VERSION,
                    directory_handoff: false,
                    features: vec![
                        FeatureGroup::Control,
                        FeatureGroup::Buffers,
                        FeatureGroup::Wait,
                    ],
                    project_root_bytes: project_root,
                    client_kind: ClientKind::Control,
                    client_version: CLIENT_VERSION.to_owned(),
                    role: ClientRole::Control,
                    geometry: FrameGeometry::default().into(),
                },
            )
            .await
            .unwrap();
            let ServerEvent::Connected { id, responses, .. } = server.recv().await.unwrap() else {
                panic!("expected connection");
            };
            responses
                .send(HostResponse::Welcome {
                    protocol: PROTOCOL_VERSION,
                    pid: std::process::id(),
                    features: vec![
                        FeatureGroup::Control,
                        FeatureGroup::Buffers,
                        FeatureGroup::Wait,
                    ],
                    host_version: CLIENT_VERSION.to_owned(),
                })
                .await
                .unwrap();
            let mut reader = MessageReader::new(reader);
            assert!(matches!(
                reader.read::<HostResponse>().await.unwrap(),
                Some(HostResponse::Welcome { .. })
            ));
            writer.write_all(payload).await.unwrap();
            writer.shutdown().await.unwrap();
            let failure = tokio::time::timeout(Duration::from_secs(1), server.recv())
                .await
                .unwrap();
            let Some(ServerEvent::TransportFailure {
                id: failed,
                message,
            }) = failure
            else {
                panic!("expected a transport failure before disconnection, got {failure:?}");
            };
            assert_eq!(failed, id);
            assert!(
                message.contains("workspace transport"),
                "the {name} framing reason must survive: {message}"
            );
            assert!(matches!(
                tokio::time::timeout(Duration::from_secs(1), server.recv())
                    .await
                    .unwrap(),
                Some(ServerEvent::Disconnected { id: disconnected }) if disconnected == id
            ));
        }
    }

    #[tokio::test]
    async fn requests_outside_the_connection_role_receive_an_error() {
        assert!(!request_allowed_for_role(
            &ClientRequest::Invoke {
                command: crate::protocol::CommandRequest::at(
                    "write",
                    crate::protocol::FrameId::from_raw(1),
                    crate::protocol::BufferId::from(crate::workspace::BufferId::from_raw(1)),
                    crate::protocol::BufferRevision::from(
                        crate::workspace::BufferRevision::from_raw(1),
                    ),
                ),
            },
            ClientRole::Control,
        ));
        assert!(!request_allowed_for_role(
            &ClientRequest::RenameHost {
                name: "renamed".to_owned(),
            },
            ClientRole::Interactive,
        ));
        assert!(request_allowed_for_role(
            &ClientRequest::Health,
            ClientRole::Interactive,
        ));
        assert!(request_allowed_for_role(
            &ClientRequest::Health,
            ClientRole::Control,
        ));
        let (root, endpoint) = endpoint("role-request");
        let Some(mut server) = bind_or_skip(&endpoint).await else {
            fs::remove_dir_all(root).unwrap();
            return;
        };
        let mut client = LocalClient::connect(&endpoint, FrameGeometry::default(), false)
            .await
            .unwrap();
        let ServerEvent::Connected { id, responses, .. } = server.recv().await.unwrap() else {
            panic!("expected connection");
        };
        responses
            .send(HostResponse::Welcome {
                protocol: PROTOCOL_VERSION,
                pid: std::process::id(),
                features: vec![
                    FeatureGroup::Control,
                    FeatureGroup::Buffers,
                    FeatureGroup::Wait,
                ],
                host_version: CLIENT_VERSION.to_owned(),
            })
            .await
            .unwrap();
        assert!(matches!(
            client.recv().await.unwrap(),
            Some(HostResponse::Welcome { .. })
        ));
        client.send(&ClientRequest::Health).await.unwrap();
        client
            .send(&ClientRequest::Input {
                event: crate::protocol::InputEvent::Text("not control input".to_owned()),
                repeated: false,
            })
            .await
            .unwrap();
        assert!(matches!(
            server.recv().await,
            Some(ServerEvent::Request {
                id: request_id,
                request: ClientRequest::Health,
            }) if request_id == id
        ));
        responses
            .send(HostResponse::Health {
                protocol: PROTOCOL_VERSION,
                pid: std::process::id(),
                interactive_attached: false,
                unsaved_buffers: 0,
                open_buffers: 0,
                pending_wait_requests: 0,
                live_terminals: 0,
                terminal_sessions: 0,
            })
            .await
            .unwrap();
        let Some(ServerEvent::ProtocolError {
            id: rejected_id,
            message,
        }) = server.recv().await
        else {
            panic!("expected role rejection after health request");
        };
        assert_eq!(rejected_id, id);
        assert!(message.contains("connection role"), "{message}");
        responses
            .send(HostResponse::Error { message })
            .await
            .unwrap();
        assert!(matches!(
            client.recv().await.unwrap(),
            Some(HostResponse::Health { .. })
        ));
        assert!(matches!(
            client.recv().await.unwrap(),
            Some(HostResponse::Error { message }) if message.contains("connection role")
        ));
        endpoint.cleanup().unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn incomplete_handshakes_cannot_grow_connection_tasks_without_bound() {
        let (root, endpoint) = endpoint("connection-bound");
        let Some(mut server) = bind_or_skip(&endpoint).await else {
            fs::remove_dir_all(root).unwrap();
            return;
        };
        let mut held = Vec::new();
        for _ in 0..MAX_CONNECTIONS {
            let mut stream = UnixStream::connect(endpoint.socket()).await.unwrap();
            stream.write_all(b"{").await.unwrap();
            held.push(stream);
        }
        tokio::time::sleep(Duration::from_millis(50)).await;

        let mut excess = UnixStream::connect(endpoint.socket()).await.unwrap();
        let mut byte = [0_u8; 1];
        assert!(
            tokio::time::timeout(Duration::from_millis(500), excess.read(&mut byte))
                .await
                .is_ok(),
            "an excess connection retained another handshake task"
        );
        drop(held.pop());
        tokio::time::sleep(Duration::from_millis(50)).await;
        let client = tokio::time::timeout(
            Duration::from_secs(1),
            LocalClient::connect(&endpoint, FrameGeometry::default(), false),
        )
        .await
        .expect("a handshake slot was not released")
        .unwrap();
        assert!(matches!(
            tokio::time::timeout(Duration::from_secs(1), server.recv())
                .await
                .unwrap(),
            Some(ServerEvent::Connected {
                interactive: false,
                ..
            })
        ));
        drop(client);
        drop(held);
        endpoint.cleanup().unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn a_peer_that_stops_reading_cannot_hold_a_write_forever() {
        let (mut writer, _reader) = tokio::io::duplex(1);
        let error = write_message_with_timeout(
            &mut writer,
            &serde_json::json!({ "payload": "x".repeat(64 * 1024) }),
            Duration::from_millis(20),
        )
        .await
        .unwrap_err()
        .to_string();
        assert!(error.contains("timed out"), "{error}");
    }

    #[tokio::test]
    async fn a_slow_but_reading_peer_receives_a_whole_message() {
        // The message is far larger than the pipe, so every write blocks
        // until the peer drains what is already there. No single write waits
        // longer than the peer's pause, while the transfer as a whole takes
        // several times the stall budget: a deadline spanning the message
        // would cut this frame in half and leave the peer reading a message
        // that ends inside itself.
        let (mut writer, mut reader) = tokio::io::duplex(512);
        let message = serde_json::json!({ "payload": "x".repeat(8 * 1024) });
        let drain = tokio::spawn(async move {
            let mut received = Vec::new();
            let mut chunk = [0_u8; 512];
            loop {
                tokio::time::sleep(Duration::from_millis(20)).await;
                let count = reader.read(&mut chunk).await.unwrap();
                if count == 0 {
                    break;
                }
                received.extend_from_slice(&chunk[..count]);
                if received.last() == Some(&b'\n') {
                    break;
                }
            }
            received
        });
        let stall = Duration::from_millis(250);
        let started = std::time::Instant::now();
        write_message_with_timeout(&mut writer, &message, stall)
            .await
            .unwrap();
        assert!(
            started.elapsed() > stall,
            "the transfer was too quick to distinguish a stall budget from a message one"
        );
        let received = drain.await.unwrap();
        assert_eq!(received.last(), Some(&b'\n'), "message was not terminated");
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&received[..received.len() - 1]).unwrap(),
            message,
            "the peer read a truncated message"
        );
    }

    #[tokio::test]
    async fn closing_the_response_channel_delivers_what_is_queued_before_disconnecting() {
        // How a shutting-down host ends an attachment without truncating it:
        // the queued responses are written first and the socket closes on a
        // frame boundary, so the peer reads a clean end of stream.
        let (root, endpoint) = endpoint("shutdown-flush");
        let Some(mut server) = bind_or_skip(&endpoint).await else {
            fs::remove_dir_all(root).unwrap();
            return;
        };
        let mut client = LocalClient::connect(&endpoint, FrameGeometry::default(), false)
            .await
            .unwrap();
        let ServerEvent::Connected { id, responses, .. } = server.recv().await.unwrap() else {
            panic!("expected connection");
        };
        responses
            .try_send(HostResponse::Error {
                message: "x".repeat(64 * 1024),
            })
            .unwrap();
        responses.try_send(HostResponse::ShuttingDown).unwrap();
        drop(responses);
        assert!(matches!(
            client.recv().await.unwrap(),
            Some(HostResponse::Error { message }) if message.len() == 64 * 1024
        ));
        assert!(matches!(
            client.recv().await.unwrap(),
            Some(HostResponse::ShuttingDown)
        ));
        assert!(
            client.recv().await.unwrap().is_none(),
            "the connection did not end on a frame boundary"
        );
        assert!(matches!(
            tokio::time::timeout(Duration::from_secs(3), server.recv())
                .await
                .expect("connection did not report its end"),
            Some(ServerEvent::Disconnected { id: disconnected }) if disconnected == id
        ));
        endpoint.cleanup().unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn an_established_stalled_peer_disconnects_and_releases_its_slot() {
        let (root, endpoint) = endpoint("write-timeout");
        let Some(mut server) = bind_or_skip(&endpoint).await else {
            fs::remove_dir_all(root).unwrap();
            return;
        };
        let stream = UnixStream::connect(endpoint.socket()).await.unwrap();
        let (reader, mut writer) = stream.into_split();
        write_message(
            &mut writer,
            &ClientRequest::Hello {
                protocol: PROTOCOL_VERSION,
                directory_handoff: false,
                features: vec![
                    FeatureGroup::Control,
                    FeatureGroup::Buffers,
                    FeatureGroup::Wait,
                ],
                project_root_bytes: encode_path(&endpoint.project_root),
                client_kind: ClientKind::Control,
                client_version: CLIENT_VERSION.to_owned(),
                role: ClientRole::Control,
                geometry: FrameGeometry::default().into(),
            },
        )
        .await
        .unwrap();
        let ServerEvent::Connected { id, responses, .. } = server.recv().await.unwrap() else {
            panic!("expected connection");
        };
        responses
            .send(HostResponse::Welcome {
                protocol: PROTOCOL_VERSION,
                pid: std::process::id(),
                features: vec![
                    FeatureGroup::Control,
                    FeatureGroup::Buffers,
                    FeatureGroup::Wait,
                ],
                host_version: CLIENT_VERSION.to_owned(),
            })
            .await
            .unwrap();
        let mut reader = MessageReader::new(reader);
        assert!(matches!(
            reader.read::<HostResponse>().await.unwrap(),
            Some(HostResponse::Welcome { .. })
        ));
        responses
            .send(HostResponse::Error {
                message: "x".repeat(7 * 1024 * 1024),
            })
            .await
            .unwrap();
        let failure = tokio::time::timeout(Duration::from_secs(3), server.recv())
            .await
            .expect("stalled write did not time out");
        let Some(ServerEvent::TransportFailure {
            id: failed,
            message,
        }) = failure
        else {
            panic!("expected a write failure before disconnection, got {failure:?}");
        };
        assert_eq!(failed, id);
        assert!(
            message.contains("workspace transport write timed out"),
            "{message}"
        );
        assert!(matches!(
            tokio::time::timeout(Duration::from_secs(1), server.recv())
                .await
                .unwrap(),
            Some(ServerEvent::Disconnected { id: disconnected }) if disconnected == id
        ));
        drop(reader);
        drop(writer);
        tokio::time::sleep(Duration::from_millis(20)).await;

        let _replacement = LocalClient::connect(&endpoint, FrameGeometry::default(), false)
            .await
            .unwrap();
        assert!(matches!(
            tokio::time::timeout(Duration::from_secs(1), server.recv())
                .await
                .unwrap(),
            Some(ServerEvent::Connected { .. })
        ));
        endpoint.cleanup().unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn a_timed_out_client_writer_is_poisoned() {
        let (writer, _reader) = tokio::io::duplex(1);
        let mut writer = Some(writer);
        let error = write_client_message(
            &mut writer,
            &serde_json::json!({ "payload": "x".repeat(64 * 1024) }),
        )
        .await
        .unwrap_err()
        .to_string();
        assert!(error.contains("timed out"), "{error}");
        assert!(writer.is_none());
        let error = write_client_message(&mut writer, &serde_json::json!({ "next": true }))
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains("writer is closed"), "{error}");
    }

    #[tokio::test]
    async fn live_recorded_process_prevents_socket_unlink_and_transient_errors_are_not_stale() {
        let (root, endpoint) = endpoint("live-process");
        let Some(server) = bind_or_skip(&endpoint).await else {
            fs::remove_dir_all(root).unwrap();
            return;
        };
        server.stop_abruptly().await;
        let error = match LocalServer::bind(&endpoint).await {
            Ok(_) => panic!("live recorded process unexpectedly allowed endpoint replacement"),
            Err(error) => error.to_string(),
        };
        assert!(error.contains("process is still alive"), "{error}");
        assert!(endpoint.socket().exists());
        assert!(endpoint.metadata().exists());
        assert!(!is_conclusive_stale(&io::Error::from(
            io::ErrorKind::Interrupted
        )));
        assert!(!is_conclusive_stale(&io::Error::from(
            io::ErrorKind::PermissionDenied
        )));
        endpoint.cleanup().unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn missing_required_feature_groups_are_refused() {
        let (root, endpoint) = endpoint("features");
        let Some(_server) = bind_or_skip(&endpoint).await else {
            fs::remove_dir_all(root).unwrap();
            return;
        };
        let stream = UnixStream::connect(endpoint.socket()).await.unwrap();
        let (reader, mut writer) = stream.into_split();
        write_message(
            &mut writer,
            &ClientRequest::Hello {
                protocol: PROTOCOL_VERSION,
                directory_handoff: false,
                features: vec![FeatureGroup::Snapshots],
                project_root_bytes: encode_path(&endpoint.project_root),
                client_kind: ClientKind::Tui,
                client_version: CLIENT_VERSION.to_owned(),
                role: ClientRole::Interactive,
                geometry: FrameGeometry::default().into(),
            },
        )
        .await
        .unwrap();
        let mut reader = MessageReader::new(reader);
        let response: HostResponse = reader.read().await.unwrap().unwrap();
        assert!(matches!(
            response,
            HostResponse::Refused { message } if message.contains("feature set")
        ));
        endpoint.cleanup().unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn a_client_advertising_extra_feature_groups_is_accepted() {
        // Role coverage is what matters, not an exact list, so a later client
        // can gain a group without the host refusing it. Order must not
        // matter either.
        let (root, endpoint) = endpoint("superset-features");
        let Some(mut server) = bind_or_skip(&endpoint).await else {
            fs::remove_dir_all(root).unwrap();
            return;
        };
        let stream = UnixStream::connect(endpoint.socket()).await.unwrap();
        let (_reader, mut writer) = stream.into_split();
        write_message(
            &mut writer,
            &ClientRequest::Hello {
                protocol: PROTOCOL_VERSION,
                directory_handoff: false,
                features: vec![
                    FeatureGroup::Wait,
                    FeatureGroup::Input,
                    FeatureGroup::Control,
                    FeatureGroup::Buffers,
                    FeatureGroup::Snapshots,
                ],
                project_root_bytes: encode_path(&endpoint.project_root),
                client_kind: ClientKind::Tui,
                client_version: CLIENT_VERSION.to_owned(),
                role: ClientRole::Interactive,
                geometry: FrameGeometry::default().into(),
            },
        )
        .await
        .unwrap();
        assert!(matches!(
            server.recv().await.unwrap(),
            ServerEvent::Connected {
                interactive: true,
                ..
            }
        ));
        drop(server);
        endpoint.cleanup().unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn oversized_wire_messages_are_rejected_before_deserialization() {
        let (mut writer, reader) = tokio::io::duplex(64 * 1024);
        let write = tokio::spawn(async move {
            let chunk = vec![b'x'; 64 * 1024];
            let mut remaining = MAX_MESSAGE_BYTES + 1;
            while remaining > 0 {
                let take = remaining.min(chunk.len());
                writer.write_all(&chunk[..take]).await.unwrap();
                remaining -= take;
            }
            writer.write_all(b"\n").await.unwrap();
        });
        let error = MessageReader::new(reader)
            .read::<serde_json::Value>()
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains("exceeds"), "{error}");
        write.abort();
    }

    #[tokio::test]
    async fn a_partially_read_message_survives_select_cancellation() {
        // Both transport loops read inside `select!`. A message larger than
        // the internal buffer arrives in pieces, and the competing branch
        // becomes ready only after the read has consumed part of it and
        // parked. Losing those bytes would resume mid-message and desync the
        // stream for the rest of the connection.
        let (mut writer, reader) = tokio::io::duplex(1024 * 1024);
        let message = serde_json::json!({ "payload": "y".repeat(200_000) });
        let encoded = serde_json::to_vec(&message).unwrap();
        let mut reader = MessageReader::new(reader);
        let (other_tx, mut other_rx) = mpsc::channel::<()>(8);

        writer.write_all(&encoded[..100_000]).await.unwrap();
        let rest = encoded[100_000..].to_vec();
        let feeder = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            other_tx.send(()).await.unwrap();
            tokio::time::sleep(Duration::from_millis(50)).await;
            writer.write_all(&rest).await.unwrap();
            writer.write_all(b"\n").await.unwrap();
            // Hold the writer so the reader never observes a premature EOF.
            tokio::time::sleep(Duration::from_secs(5)).await;
        });

        let mut cancellations = 0;
        let received = loop {
            tokio::select! {
                Some(()) = other_rx.recv() => cancellations += 1,
                result = reader.read::<serde_json::Value>() => break result.unwrap().unwrap(),
            }
        };
        assert_eq!(cancellations, 1, "the read was not actually cancelled");
        assert_eq!(received, message);
        feeder.abort();
    }

    #[tokio::test]
    async fn framing_stays_aligned_across_repeated_cancellation() {
        // Every message must still be delivered, in order, when the reader is
        // cancelled repeatedly while messages stream in.
        let (mut writer, reader) = tokio::io::duplex(1024 * 1024);
        let mut reader = MessageReader::new(reader);
        let expected = (0..8)
            .map(|index| serde_json::json!({ "index": index, "pad": "z".repeat(30_000) }))
            .collect::<Vec<_>>();
        let payload = expected.clone();
        let feeder = tokio::spawn(async move {
            for message in payload {
                writer
                    .write_all(&serde_json::to_vec(&message).unwrap())
                    .await
                    .unwrap();
                writer.write_all(b"\n").await.unwrap();
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
            tokio::time::sleep(Duration::from_secs(5)).await;
        });

        let mut interrupts = tokio::time::interval(Duration::from_micros(200));
        let mut received = Vec::new();
        while received.len() < expected.len() {
            tokio::select! {
                _ = interrupts.tick() => {}
                result = reader.read::<serde_json::Value>() => {
                    received.push(result.unwrap().unwrap());
                }
            }
        }
        assert_eq!(received, expected);
        feeder.abort();
    }
}
