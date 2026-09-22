// SPDX-License-Identifier: MPL-2.0

//! Bounded native control exchanges and authenticated process termination.
//! Configured publication locations, discovery and detached startup remain
//! separate owners. No operation below derives authority from a metadata PID.

use std::{
    io,
    os::windows::io::{AsHandle, AsRawHandle, FromRawHandle, OwnedHandle},
    sync::Arc,
    time::Duration,
};

use anyhow::{Context, Result, ensure};
use tokio::time::{Instant, sleep_until, timeout_at};
use windows_sys::Win32::{
    Foundation::CompareObjectHandles,
    System::Threading::{
        GetCurrentProcess, OpenProcess, PROCESS_SYNCHRONIZE, PROCESS_TERMINATE, TerminateProcess,
    },
};

use super::{
    session_name::validate_host_name,
    windows_endpoint::{AuthenticatedHost, Candidate, EndpointMetadata, Inspection, Removal},
    windows_process_identity::{PinnedProcess, ProcessIdentity},
    windows_transport::LocalClient,
};
use crate::{
    app::FrameGeometry,
    protocol::{ClientRequest, HostResponse, validate_welcome},
};

const CONTROL_BUDGET: Duration = Duration::from_secs(2);
const STOP_BUDGET: Duration = Duration::from_secs(5);
const STOP_POLL_INTERVAL: Duration = Duration::from_millis(25);

/// A stop targets one authenticated native process and one publication. The
/// retained handle cannot become a replacement process when its PID is reused.
/// A receipt retains the target after a stop request. A protocol stop can return
/// it on acknowledgment or EOF; only `await_host_stopped` confirms process exit.
#[derive(Debug)]
pub struct StopReceipt {
    metadata: EndpointMetadata,
    peer: Arc<PinnedProcess>,
}

impl StopReceipt {
    pub fn metadata(&self) -> &EndpointMetadata {
        &self.metadata
    }

    pub fn process(&self) -> ProcessIdentity {
        self.peer.identity()
    }
}

/// Authenticates the actual pipe server and completes the compatible control
/// handshake within one budget, including any pipe availability retry or Hello
/// write. A prior discovery probe does not replace this connection's proof.
pub async fn connect_control(metadata: &EndpointMetadata) -> Result<LocalClient> {
    timeout_at(Instant::now() + CONTROL_BUDGET, async {
        let mut client = LocalClient::connect(metadata, FrameGeometry::default(), false).await?;
        match client.recv().await? {
            Some(response @ HostResponse::Welcome { .. }) => {
                validate_welcome(&response, false).map_err(anyhow::Error::msg)?;
                if let HostResponse::Welcome { pid, .. } = response {
                    ensure!(
                        pid == client.peer().identity().pid,
                        "workspace welcome does not match its authenticated native peer"
                    );
                }
                Ok(client)
            }
            Some(HostResponse::Refused { message } | HostResponse::Error { message }) => {
                anyhow::bail!(message)
            }
            Some(response) => {
                anyhow::bail!("unexpected workspace handshake response: {response:?}")
            }
            None => anyhow::bail!("workspace host disconnected during handshake"),
        }
    })
    .await
    .context("workspace host handshake timed out")?
}

/// Only the host publication owner changes live names or persisted name files.
/// Input normalization belongs to the caller; this preserves the requested name.
pub async fn rename_host(metadata: &EndpointMetadata, name: &str) -> Result<()> {
    validate_host_name(name)?;
    let mut client = connect_control(metadata).await?;
    timeout_at(Instant::now() + CONTROL_BUDGET, async {
        client
            .send(&ClientRequest::RenameHost {
                name: name.to_owned(),
            })
            .await?;
        match client.recv().await? {
            Some(HostResponse::HostRenamed { name: renamed }) if renamed == name => Ok(()),
            Some(HostResponse::Refused { message } | HostResponse::Error { message }) => {
                anyhow::bail!(message)
            }
            Some(response) => anyhow::bail!("unexpected host-rename response: {response:?}"),
            None => anyhow::bail!("workspace host disconnected while being renamed"),
        }
    })
    .await
    .context("workspace host rename request timed out")?
}

/// The compatible host owns refusal while protected live state remains.
pub async fn shutdown_host(metadata: &EndpointMetadata) -> Result<StopReceipt> {
    shutdown_request(metadata, ClientRequest::Shutdown, "shutdown").await
}

/// Explicit compatible force remains a protocol request, never native killing.
pub async fn force_shutdown_host(metadata: &EndpointMetadata) -> Result<StopReceipt> {
    shutdown_request(metadata, ClientRequest::ForceShutdown, "force-shutdown").await
}

async fn shutdown_request(
    metadata: &EndpointMetadata,
    request: ClientRequest,
    description: &str,
) -> Result<StopReceipt> {
    let mut client = connect_control(metadata).await?;
    timeout_at(Instant::now() + CONTROL_BUDGET, async {
        client.send(&request).await?;
        match client.recv().await? {
            Some(HostResponse::ShuttingDown) | None => Ok(StopReceipt {
                metadata: metadata.clone(),
                peer: client.peer().clone(),
            }),
            Some(HostResponse::Refused { message } | HostResponse::Error { message }) => {
                anyhow::bail!(message)
            }
            Some(response) => anyhow::bail!("unexpected {description} response: {response:?}"),
        }
    })
    .await
    .with_context(|| format!("workspace host {description} request timed out"))?
}

/// Waits only for the original process. Ready-record disappearance can occur
/// during live name publication and is not proof of process termination. This
/// finite active-operation poll introduces no timer into an idle editor.
pub async fn await_host_stopped(receipt: &StopReceipt) -> Result<()> {
    await_host_stopped_until(receipt, Instant::now() + STOP_BUDGET).await
}

async fn await_host_stopped_until(receipt: &StopReceipt, deadline: Instant) -> Result<()> {
    loop {
        if !receipt.peer.is_alive()? {
            return Ok(());
        }
        ensure!(
            Instant::now() < deadline,
            "workspace host did not finish shutting down"
        );
        sleep_until(deadline.min(Instant::now() + STOP_POLL_INTERVAL)).await;
    }
}

/// Explicit recovery for an incompatible host. A private observation alone is
/// insufficient: the actual pipe peer must still authenticate before native
/// termination authority is opened. Compatible hosts must use their protocol.
pub async fn terminate_incompatible_host(candidate: &Candidate) -> Result<StopReceipt> {
    ensure!(
        candidate.metadata().protocol != crate::protocol::VERSION,
        "this persistent session speaks the current protocol; stop it with --session-stop so its protected state is respected"
    );
    let host = candidate
        .authenticate(Instant::now() + CONTROL_BUDGET)
        .await
        .context("cannot authenticate incompatible workspace host")?;
    request_authenticated_exit(&host)?;
    let receipt = StopReceipt {
        metadata: host.metadata().clone(),
        peer: host.peer().clone(),
    };
    await_host_stopped(&receipt).await?;
    Ok(receipt)
}

/// Filesystem cleanup stays explicit and exact. An inventory candidate grants
/// only its observed row's cleanup scope, never an inferred namespace or ready
/// path. Call from lifecycle work off the editor loop, as with discovery.
pub fn remove_stopped_observation(receipt: &StopReceipt, candidate: &Candidate) -> Result<Removal> {
    let observed = candidate.metadata();
    let stopped = &receipt.metadata;
    ensure!(
        observed.process == stopped.process
            && observed.id == stopped.id
            && observed.incarnation == stopped.incarnation
            && observed.address == stopped.address
            && observed.project_root_bytes == stopped.project_root_bytes,
        "workspace observation does not belong to the stopped publication"
    );
    ensure!(!receipt.peer.is_alive()?, "workspace host is still running");
    match candidate.inspect_process()? {
        Inspection::Stale(evidence) => Ok(evidence.remove_observed()?),
        Inspection::Present(_) => anyhow::bail!("workspace observation still has a live process"),
    }
}

fn request_authenticated_exit(host: &AuthenticatedHost) -> io::Result<()> {
    // AuthenticatedHost has private fields and is only constructed from actual
    // pipe admission. In particular, PinnedProcess::open(metadata) is not an
    // alternate route into this capability.
    if !host.peer().is_alive()? {
        return Ok(());
    }
    let original = host.peer().as_handle();
    if unsafe { CompareObjectHandles(original.as_raw_handle(), GetCurrentProcess()) } != 0 {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "refusing to terminate the current lifecycle client",
        ));
    }
    let handle = unsafe {
        OpenProcess(
            PROCESS_TERMINATE | PROCESS_SYNCHRONIZE,
            0,
            host.peer().identity().pid,
        )
    };
    if handle.is_null() {
        let error = io::Error::last_os_error();
        return if host.peer().is_alive()? {
            Err(error)
        } else {
            Ok(())
        };
    }
    let process = unsafe { OwnedHandle::from_raw_handle(handle) };
    terminate_matching_handle(host, &process)
}

fn terminate_matching_handle(host: &AuthenticatedHost, process: &OwnedHandle) -> io::Result<()> {
    if unsafe {
        CompareObjectHandles(
            host.peer().as_handle().as_raw_handle(),
            process.as_raw_handle(),
        )
    } == 0
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "termination handle is not the authenticated native peer",
        ));
    }
    if unsafe { TerminateProcess(process.as_raw_handle(), 1) } == 0 {
        let error = io::Error::last_os_error();
        // A concurrent natural exit is complete; access denial while the
        // original process remains live is still an error, never stale proof.
        if host.peer().is_alive()? {
            return Err(error);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
