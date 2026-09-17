// SPDX-License-Identifier: MPL-2.0

//! Metadata-only discovery: private registrations are hints until a bounded
//! same-user probe confirms the exact live host. This never attaches or grants.

use super::storage::{Registration, Storage};
use futures_util::{StreamExt, stream};
use serde::{Deserialize, Serialize};
use std::{
    io,
    os::unix::fs::{FileTypeExt, MetadataExt},
    path::PathBuf,
    time::Duration,
};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    net::UnixStream,
};

pub const SCHEMA: &str = "runyte.context.discovery.v1";
const MAX_ENDPOINTS: usize = 64;
const CONCURRENCY: usize = 8;
const PROBE_BYTES: usize = 64 * 1024;
const PROBE_TIMEOUT: Duration = Duration::from_millis(250);
const DISCOVERY_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Debug, Serialize)]
pub struct Discovery {
    pub schema: &'static str,
    /// True means a bounded scan did not probe every admitted registration.
    pub truncated: bool,
    pub workspaces: Vec<Endpoint>,
}

#[derive(Debug, Serialize)]
pub struct Endpoint {
    #[serde(flatten)]
    pub registration: Registration,
    pub label: String,
    pub transport_version: &'static str,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Probe {
    #[serde(rename = "type")]
    kind: String,
    registration: Registration,
}

/// Missing storage is the normal disabled state and creates nothing. Existing
/// malformed/insecure inventories fail explicitly rather than appearing empty.
pub async fn discover(
    root: Option<PathBuf>,
    environment: &str,
    include_hidden: bool,
) -> io::Result<Discovery> {
    let mut result = Discovery {
        schema: SCHEMA,
        truncated: false,
        workspaces: Vec::new(),
    };
    let Some(root) = root else {
        return Ok(result);
    };
    let storage = match Storage::open_existing(root) {
        Ok(storage) => storage,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(result),
        Err(error) => return Err(error),
    };
    let registrations = storage.discover(environment, include_hidden)?;
    result.truncated = registrations.len() > MAX_ENDPOINTS;
    let probes = stream::iter(registrations.into_iter().take(MAX_ENDPOINTS).map(
        |registration| async move {
            let valid = matches!(
                tokio::time::timeout(PROBE_TIMEOUT, probe(&registration)).await,
                Ok(Ok(()))
            );
            valid.then_some(registration)
        },
    ))
    .buffer_unordered(CONCURRENCY);
    tokio::pin!(probes);
    let deadline = tokio::time::Instant::now() + DISCOVERY_TIMEOUT;
    loop {
        match tokio::time::timeout_at(deadline, probes.next()).await {
            Ok(Some(Some(registration))) => {
                let label = registration
                    .root
                    .file_name()
                    .unwrap_or(registration.root.as_os_str())
                    .to_string_lossy()
                    .into_owned();
                result.workspaces.push(Endpoint {
                    registration,
                    label,
                    transport_version: super::wire::FEATURE,
                });
            }
            Ok(Some(None)) => {}
            Ok(None) => break,
            Err(_) => {
                result.truncated = true;
                break;
            }
        }
    }
    result.workspaces.sort_by(|a, b| {
        a.registration
            .host_incarnation
            .cmp(&b.registration.host_incarnation)
    });
    Ok(result)
}

async fn probe(registration: &Registration) -> io::Result<()> {
    let metadata = std::fs::symlink_metadata(&registration.endpoint)?;
    // SAFETY: geteuid has no preconditions.
    let owner = unsafe { libc::geteuid() };
    if !metadata.file_type().is_socket() || metadata.uid() != owner || metadata.mode() & 0o077 != 0
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "context endpoint is not an owner-private socket",
        ));
    }
    let mut socket = UnixStream::connect(&registration.endpoint).await?;
    let peer = socket.peer_cred()?;
    if peer.uid() != owner || peer.pid().is_some_and(|pid| pid as u32 != registration.pid) {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "context endpoint peer does not match registration",
        ));
    }
    socket.write_all(b"{\"type\":\"probe\"}\n").await?;
    let mut bytes = Vec::new();
    BufReader::new(socket)
        .take((PROBE_BYTES + 1) as u64)
        .read_until(b'\n', &mut bytes)
        .await?;
    if bytes.len() > PROBE_BYTES || bytes.last() != Some(&b'\n') {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid context probe frame",
        ));
    }
    let response: Probe = serde_json::from_slice(&bytes).map_err(|_| {
        io::Error::new(io::ErrorKind::InvalidData, "invalid context probe response")
    })?;
    if response.kind != "context_endpoint" || &response.registration != registration {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "context endpoint registration changed",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::storage::{HostMode, random_token};
    use super::*;
    use std::{
        os::unix::fs::{PermissionsExt, symlink},
        sync::atomic::{AtomicU64, Ordering},
    };
    use tokio::net::UnixListener;

    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let root = PathBuf::from(format!(
                "/tmp/rydisc-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir_all(root.join("project")).unwrap();
            Self(root.canonicalize().unwrap())
        }
        fn store(&self) -> Storage {
            Storage::new(self.0.join("store")).unwrap()
        }
        fn registration(&self, storage: &Storage) -> Registration {
            let root = self.0.join("project");
            let host_incarnation = random_token().unwrap();
            Registration {
                workspace_id: crate::workspace::workspace_id(&root),
                root,
                endpoint: storage.socket_path(&host_incarnation).unwrap(),
                host_incarnation,
                mode: HostMode::Persistent,
                pid: std::process::id(),
                environment: "a".repeat(64),
            }
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn serve(registration: &Registration, response: Vec<u8>) -> tokio::task::JoinHandle<()> {
        let listener = UnixListener::bind(&registration.endpoint).unwrap();
        std::fs::set_permissions(
            &registration.endpoint,
            std::fs::Permissions::from_mode(0o600),
        )
        .unwrap();
        tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut reader = BufReader::new(stream);
            let mut request = String::new();
            reader.read_line(&mut request).await.unwrap();
            assert_eq!(request, "{\"type\":\"probe\"}\n");
            let _ = reader.get_mut().write_all(&response).await;
        })
    }
    fn response(registration: &Registration) -> Vec<u8> {
        let mut bytes = serde_json::to_vec(
            &serde_json::json!({"type":"context_endpoint", "registration":registration}),
        )
        .unwrap();
        bytes.push(b'\n');
        bytes
    }

    #[tokio::test]
    async fn disabled_discovery_creates_nothing() {
        let fixture = Fixture::new();
        let missing = fixture.0.join("missing");
        let result = discover(Some(missing.clone()), &"a".repeat(64), false)
            .await
            .unwrap();
        assert_eq!(result.schema, SCHEMA);
        assert!(!result.truncated);
        assert!(result.workspaces.is_empty());
        assert!(!missing.exists());
        assert!(
            discover(None, &"a".repeat(64), false)
                .await
                .unwrap()
                .workspaces
                .is_empty()
        );
    }

    #[tokio::test]
    async fn only_matching_live_incarnations_are_returned() {
        let fixture = Fixture::new();
        let store = fixture.store();
        let live = fixture.registration(&store);
        store.register(&live).unwrap();
        let server = serve(&live, response(&live));
        let stale = fixture.registration(&store);
        store.register(&stale).unwrap();
        let mismatched = fixture.registration(&store);
        store.register(&mismatched).unwrap();
        let mut replaced = mismatched.clone();
        replaced.host_incarnation = random_token().unwrap();
        let wrong = serve(&mismatched, response(&replaced));
        // Discovery reads no workspace paths, even when the original directory
        // vanished after registration. Only the live identity probe matters.
        std::fs::remove_dir(&live.root).unwrap();
        let result = discover(Some(store.root().to_owned()), &live.environment, false)
            .await
            .unwrap();
        assert_eq!(result.workspaces.len(), 1);
        assert_eq!(result.workspaces[0].registration, live);
        assert_eq!(result.workspaces[0].label, "project");
        assert_eq!(
            result.workspaces[0].transport_version,
            super::super::wire::FEATURE
        );
        let json = serde_json::to_string(&result).unwrap();
        assert!(!json.contains("credential"));
        assert!(!json.contains("scopes"));
        assert!(
            store
                .root()
                .join(format!("host-{}.json", stale.host_incarnation))
                .exists()
        );
        server.await.unwrap();
        wrong.await.unwrap();
    }

    #[tokio::test]
    async fn hidden_environment_requires_explicit_inventory_expansion() {
        let fixture = Fixture::new();
        let store = fixture.store();
        let hidden = fixture.registration(&store);
        store.register(&hidden).unwrap();
        let server = serve(&hidden, response(&hidden));
        assert!(
            discover(Some(store.root().to_owned()), &"b".repeat(64), false)
                .await
                .unwrap()
                .workspaces
                .is_empty()
        );
        assert_eq!(
            discover(Some(store.root().to_owned()), &"b".repeat(64), true)
                .await
                .unwrap()
                .workspaces
                .len(),
            1
        );
        server.await.unwrap();
    }

    #[tokio::test]
    async fn unsafe_and_malformed_endpoints_are_skipped() {
        let fixture = Fixture::new();
        let store = fixture.store();
        let linked = fixture.registration(&store);
        store.register(&linked).unwrap();
        symlink(fixture.0.join("absent"), &linked.endpoint).unwrap();
        let regular = fixture.registration(&store);
        store.register(&regular).unwrap();
        std::fs::write(&regular.endpoint, b"not a socket").unwrap();
        let public = fixture.registration(&store);
        store.register(&public).unwrap();
        let listener = UnixListener::bind(&public.endpoint).unwrap();
        std::fs::set_permissions(&public.endpoint, std::fs::Permissions::from_mode(0o666)).unwrap();
        let malformed = fixture.registration(&store);
        store.register(&malformed).unwrap();
        let bad = serve(&malformed, b"{bad json}\n".to_vec());
        let oversized = fixture.registration(&store);
        store.register(&oversized).unwrap();
        let huge = serve(&oversized, vec![b'x'; PROBE_BYTES + 1]);
        assert!(
            discover(Some(store.root().to_owned()), &"a".repeat(64), true)
                .await
                .unwrap()
                .workspaces
                .is_empty()
        );
        bad.await.unwrap();
        huge.await.unwrap();
        drop(listener);
    }

    #[tokio::test]
    async fn silent_probe_is_bounded_by_timeout() {
        let fixture = Fixture::new();
        let store = fixture.store();
        let silent = fixture.registration(&store);
        store.register(&silent).unwrap();
        let listener = UnixListener::bind(&silent.endpoint).unwrap();
        std::fs::set_permissions(&silent.endpoint, std::fs::Permissions::from_mode(0o600)).unwrap();
        let live = fixture.registration(&store);
        store.register(&live).unwrap();
        let server = serve(&live, response(&live));
        let result = tokio::time::timeout(
            Duration::from_secs(1),
            discover(Some(store.root().to_owned()), &silent.environment, false),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(result.workspaces.len(), 1);
        assert_eq!(result.workspaces[0].registration, live);
        server.await.unwrap();
        drop(listener);
    }
}
