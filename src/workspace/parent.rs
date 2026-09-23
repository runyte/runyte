// SPDX-License-Identifier: MPL-2.0

//! Private integrated-terminal launch context. A marker identifies a candidate
//! parent; the host also verifies the live terminal and the authenticated local
//! peer before it accepts a request.

#[cfg(windows)]
use crate::workspace::windows_endpoint::EndpointMetadata;
use crate::{hash::sha256_hex, terminal::TerminalId};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
#[cfg(unix)]
use std::{
    io::Read,
    path::{Path, PathBuf},
};
#[cfg(windows)]
use std::{path::PathBuf, time::Duration};

pub const ENVIRONMENT: &str = "RUNYTE_PARENT_CONTEXT";

#[derive(Clone)]
pub struct ParentLaunch {
    #[cfg(unix)]
    metadata: PathBuf,
    #[cfg(windows)]
    metadata: EndpointMetadata,
    secret: String,
}

impl std::fmt::Debug for ParentLaunch {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ParentLaunch")
            .field("metadata", &self.metadata)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ParentContext {
    pub protocol: u32,
    #[cfg(unix)]
    pub metadata: Vec<u8>,
    #[cfg(windows)]
    pub metadata: EndpointMetadata,
    pub terminal: u64,
    pub capability: String,
}

impl std::fmt::Debug for ParentContext {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ParentContext")
            .field("protocol", &self.protocol)
            .field("terminal", &self.terminal)
            .finish_non_exhaustive()
    }
}

impl ParentLaunch {
    #[cfg(unix)]
    pub fn new(metadata: &Path) -> Result<Self> {
        let mut random = [0; 32];
        std::fs::File::open("/dev/urandom")?.read_exact(&mut random)?;
        Ok(Self {
            metadata: metadata.to_owned(),
            secret: sha256_hex(&random),
        })
    }

    #[cfg(windows)]
    pub fn new(metadata: EndpointMetadata) -> Result<Self> {
        use windows_sys::Win32::Security::Cryptography::{
            BCRYPT_USE_SYSTEM_PREFERRED_RNG, BCryptGenRandom,
        };
        metadata.validate()?;
        let mut random = [0; 32];
        let status = unsafe {
            BCryptGenRandom(
                std::ptr::null_mut(),
                random.as_mut_ptr(),
                random.len() as u32,
                BCRYPT_USE_SYSTEM_PREFERRED_RNG,
            )
        };
        ensure!(status >= 0, "system random generator failed ({status:#x})");
        Ok(Self {
            metadata,
            secret: sha256_hex(&random),
        })
    }

    fn capability(&self, id: TerminalId) -> String {
        sha256_hex(format!("{}:{}", self.secret, id.get()).as_bytes())
    }

    pub fn context(&self, id: TerminalId) -> String {
        serde_json::to_string(&ParentContext {
            protocol: crate::protocol::VERSION,
            #[cfg(unix)]
            metadata: crate::protocol::encode_path(&self.metadata),
            #[cfg(windows)]
            metadata: self.metadata.clone(),
            terminal: id.get(),
            capability: self.capability(id),
        })
        .expect("parent launch context contains serializable scalar fields")
    }

    pub fn validates(&self, terminal: TerminalId, capability: &str) -> bool {
        self.capability(terminal) == capability
    }
}

impl ParentContext {
    pub fn from_environment() -> Result<Option<Self>> {
        let Some(value) = std::env::var_os(ENVIRONMENT) else {
            return Ok(None);
        };
        let value = value
            .to_str()
            .context("Runyte parent context is not valid text")?;
        Self::parse(value).map(Some)
    }

    fn parse(value: &str) -> Result<Self> {
        ensure!(
            value != "standalone",
            "this integrated terminal belongs to a standalone editor; launch a persistent session outside it first"
        );
        ensure!(
            value.len() <= 65536,
            "Runyte parent context exceeds its limit"
        );
        let context: Self = serde_json::from_str(value)
            .context("invalid Runyte parent context; open a fresh integrated terminal")?;
        ensure!(
            context.protocol == crate::protocol::VERSION,
            "Runyte parent context uses an incompatible protocol; restart the persistent session"
        );
        ensure!(
            context.terminal > 0
                && context.capability.len() == 64
                && context
                    .capability
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
            "invalid Runyte parent context; open a fresh integrated terminal"
        );
        #[cfg(unix)]
        ensure!(
            !context.metadata.is_empty()
                && context.metadata.len() <= crate::protocol::MAX_PATH_BYTES,
            "invalid Runyte parent context; open a fresh integrated terminal"
        );
        #[cfg(windows)]
        {
            context
                .metadata
                .validate()
                .context("invalid Runyte parent endpoint; open a fresh integrated terminal")?;
            ensure!(
                context.metadata.protocol == crate::protocol::VERSION,
                "Runyte parent endpoint uses an incompatible protocol; restart the persistent session"
            );
        }
        Ok(context)
    }
}

/// Runs the native ParentWait relay over the one exact endpoint carried by an
/// integrated terminal marker. Public `--wait` calls this only after early
/// parent-context routing; an ordinary Windows shell retains standalone wait.
#[cfg(windows)]
pub async fn run_wait(
    context: ParentContext,
    paths: Vec<PathBuf>,
    parent: &crate::workspace::windows_parent_identity::ForegroundParentSupervisor,
) -> Result<()> {
    use crate::{
        protocol::{ClientRequest, HostResponse, WaitStatus},
        workspace::windows_lifecycle::connect_control,
    };
    use tokio::time::{Instant, timeout_at};

    parent.ensure_alive()?;
    // Connect, Hello, Welcome, ParentWait and WaitCreated share one admission
    // window. connect_control also binds Welcome to the retained pipe peer.
    let deadline = Instant::now() + Duration::from_secs(3);
    let mut client = tokio::select! {
        biased;
        _ = parent.wait() => anyhow::bail!("supervising parent exited before parent editing was admitted"),
        connected = timeout_at(deadline, connect_control(&context.metadata)) => {
            connected.context("owning native Runyte host did not answer")?
                .context("cannot connect to owning native Runyte host")?
        }
    };
    let request = ClientRequest::ParentWait {
        terminal: context.terminal,
        capability: context.capability,
        paths: paths
            .iter()
            .map(|path| crate::protocol::encode_path(path))
            .collect(),
    };
    tokio::select! {
        biased;
        _ = parent.wait() => anyhow::bail!("supervising parent exited before parent editing was admitted"),
        sent = timeout_at(deadline, client.send(&request)) => {
            sent.context("parent editor did not accept the request")??;
        }
    }
    // Until WaitCreated supplies a token, dropping this exact connection is
    // the cancellation path: the host cancels its waits on peer disconnect.
    let admitted = tokio::select! {
        biased;
        _ = parent.wait() => anyhow::bail!("supervising parent exited before parent editing was admitted"),
        response = timeout_at(deadline, client.recv()) => {
            response.context("parent editor did not accept the request")??
        }
    };
    let token = match admitted {
        Some(HostResponse::WaitCreated { token, .. }) => token,
        Some(HostResponse::Error { message } | HostResponse::Refused { message }) => {
            anyhow::bail!(message)
        }
        _ => anyhow::bail!("owning host did not create a parent editor request"),
    };

    loop {
        tokio::select! {
            biased;
            _ = parent.wait() => {
                return cancel_after_parent_loss(&mut client, token).await;
            }
            response = client.recv() => match response? {
                Some(HostResponse::WaitState { token: received, status: WaitStatus::Completed, .. }) if received == token => return Ok(()),
                Some(HostResponse::WaitState { token: received, status: WaitStatus::Cancelled { reason }, .. }) if received == token => anyhow::bail!(reason),
                Some(HostResponse::Error { message } | HostResponse::Refused { message }) => anyhow::bail!(message),
                Some(_) => {}
                None => anyhow::bail!("owning native Runyte host disconnected before wait completion"),
            },
        }
    }
}

/// Runs a native ParentAttach relay over the exact source endpoint and retained
/// terminal authority, including an explicit `-a` from an integrated terminal.
#[cfg(windows)]
pub async fn run_attach(
    context: ParentContext,
    selector: PathBuf,
    directory: PathBuf,
    parent: &crate::workspace::windows_parent_identity::ForegroundParentSupervisor,
) -> Result<()> {
    use crate::{
        protocol::{ClientRequest, HostResponse},
        workspace::windows_lifecycle::connect_control,
    };
    use tokio::time::{Instant, timeout_at};

    parent.ensure_alive()?;
    // Connection admission, destination preparation, the source frontend
    // handshake and the final child reply share one whole-operation budget.
    // Dropping this exact connection on parent loss revokes the retained child
    // proof and makes the source host abort any provisional destination.
    // The host admits and completes the handoff within fifteen seconds. Keep
    // the child connection alive for its separate five-second cleanup
    // allowance so an owned provisional startup can settle and return one
    // terminal result instead of becoming an ambiguous timeout.
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut client = tokio::select! {
        biased;
        _ = parent.wait() => anyhow::bail!("supervising parent exited before parent attach was admitted"),
        connected = timeout_at(deadline, connect_control(&context.metadata)) => {
            connected.context("owning native Runyte host did not answer")?
                .context("cannot connect to owning native Runyte host")?
        }
    };
    let request = ClientRequest::ParentAttach {
        terminal: context.terminal,
        capability: context.capability,
        selector: crate::protocol::encode_path(&selector),
        directory: crate::protocol::encode_path(&directory),
    };
    tokio::select! {
        biased;
        _ = parent.wait() => anyhow::bail!("supervising parent exited before parent attach was admitted"),
        sent = timeout_at(deadline, client.send(&request)) => {
            sent.context("parent editor did not accept the attach request")??;
        }
    }
    loop {
        let response = tokio::select! {
            biased;
            _ = parent.wait() => anyhow::bail!("supervising parent exited before parent attach completed"),
            response = timeout_at(deadline, client.recv()) => {
                response.context("parent attach exceeded its whole-operation deadline")??
            }
        };
        match response {
            Some(HostResponse::ParentAttached) => return Ok(()),
            Some(HostResponse::Error { message } | HostResponse::Refused { message }) => {
                anyhow::bail!(message)
            }
            Some(_) => {}
            None => anyhow::bail!(
                "owning native Runyte host disconnected before parent attach completed"
            ),
        }
    }
}

#[cfg(windows)]
async fn cancel_after_parent_loss(
    client: &mut crate::workspace::windows_transport::LocalClient,
    token: crate::protocol::WaitToken,
) -> Result<()> {
    use crate::protocol::{ClientRequest, HostResponse, WaitStatus};
    use tokio::time::{Instant, timeout_at};

    // The write and same-reader recovery consume one bounded window. A timed
    // out or failed send poisons the writer; never retry an uncertain request.
    let deadline = Instant::now() + Duration::from_secs(3);
    let write_error =
        match timeout_at(deadline, client.send(&ClientRequest::CancelWait { token })).await {
            Ok(result) => result.err(),
            Err(error) => Some(error.into()),
        };
    loop {
        let response = timeout_at(deadline, client.recv())
            .await
            .context("parent-loss cancellation exceeded its bounded recovery window")??;
        match response {
            Some(HostResponse::WaitState {
                token: received,
                status: WaitStatus::Completed,
                ..
            }) if received == token => return Ok(()),
            Some(HostResponse::WaitState {
                token: received,
                status: WaitStatus::Cancelled { reason },
                ..
            }) if received == token => anyhow::bail!(reason),
            Some(HostResponse::Error { message } | HostResponse::Refused { message }) => {
                anyhow::bail!(message)
            }
            Some(_) => {}
            None => {
                if let Some(error) = write_error {
                    return Err(error).context(
                        "parent exited and the owning host closed before cancellation was observed",
                    );
                }
                anyhow::bail!(
                    "parent exited and the owning host closed before cancellation was observed"
                )
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    #[test]
    fn native_parent_context_captures_exact_endpoint_and_redacts_capability() {
        let root = crate::test_support::TestRuntimeRoot::new("native-parent-context").unwrap();
        let location = crate::workspace::windows_endpoint::EndpointLocation::new(
            root.path(),
            root.join("endpoint"),
            crate::workspace::windows_endpoint::RegistrySet::open(&[root.join("registry")])
                .unwrap(),
        )
        .unwrap();
        let prepared = location.prepare(None).unwrap();
        let metadata = prepared.metadata().clone();
        drop(prepared);
        let launch = ParentLaunch::new(metadata.clone()).unwrap();
        let terminal = TerminalId::from_raw(7);
        let context = ParentContext::parse(&launch.context(terminal)).unwrap();
        assert_eq!(context.metadata, metadata);
        assert!(launch.validates(terminal, &context.capability));
        assert!(!launch.validates(TerminalId::from_raw(8), &context.capability));
        assert!(!format!("{launch:?}").contains(&launch.secret));
        assert!(!format!("{context:?}").contains(&context.capability));
    }

    #[cfg(unix)]
    #[test]
    fn parent_context_is_bound_to_host_and_terminal_and_never_debugs_secret() {
        let launch = ParentLaunch::new(Path::new("/tmp/runyte-parent-test/endpoint.json")).unwrap();
        let id = TerminalId::from_raw(1);
        let context = ParentContext::parse(&launch.context(id)).unwrap();
        assert!(launch.validates(id, &context.capability));
        assert!(!launch.validates(TerminalId::from_raw(2), &context.capability));
        let replaced =
            ParentLaunch::new(Path::new("/tmp/runyte-parent-test/endpoint.json")).unwrap();
        assert!(!replaced.validates(id, &context.capability));
        assert!(!format!("{launch:?}").contains(&launch.secret));
        assert!(
            ParentContext::parse("standalone")
                .unwrap_err()
                .to_string()
                .contains("standalone")
        );
        assert!(ParentContext::parse("not-json").is_err());
        assert!(ParentContext::parse(&"x".repeat(65537)).is_err());
        let mut stale = context.clone();
        stale.protocol = 0;
        assert!(ParentContext::parse(&serde_json::to_string(&stale).unwrap()).is_err());
        stale = context;
        stale.terminal = 0;
        assert!(ParentContext::parse(&serde_json::to_string(&stale).unwrap()).is_err());
    }
    #[cfg(unix)]
    #[test]
    fn copied_parent_capability_cannot_authorize_another_unix_session() {
        let launch = ParentLaunch::new(Path::new("/tmp/runyte-parent-test/endpoint.json")).unwrap();
        let mut terminals = crate::terminal::TerminalSessions::new();
        terminals.set_parent_launch(launch.clone());
        let id = terminals
            .open(
                crate::terminal::TerminalRequest {
                    program: "/bin/cat".into(),
                    arguments: vec![],
                    directory: std::env::temp_dir(),
                    label: "parent".to_owned(),
                },
                80,
                24,
            )
            .unwrap();
        let context = ParentContext::parse(&launch.context(id)).unwrap();
        let process = terminals.get(id).unwrap().process_id();
        assert!(terminals.validates_parent(id, &context.capability, process));
        assert!(!terminals.validates_parent(id, &context.capability, Some(std::process::id())));
        assert!(!terminals.validates_parent(id, &context.capability, None));
        terminals.close(id);
        assert!(!terminals.validates_parent(id, &context.capability, process));
    }
}
