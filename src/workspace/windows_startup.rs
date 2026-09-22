// SPDX-License-Identifier: MPL-2.0

//! Provisional native host ownership and authenticated detached handoff.
//! No CLI/frontend is enabled here. Run this lifecycle on its background owner:
//! filesystem observations and native CreateProcess are synchronous operations.
//! Successful release changes only the private startup job. Windows may retain
//! restrictive outer jobs even when explicit breakaway succeeds, so this does
//! not guarantee survival of an external supervisor's job teardown. Known
//! Runyte ConPTY launches must route through their parent host in later wiring.

use super::{
    windows_endpoint::{Candidate, EndpointLocation, EndpointMetadata, Inspection, Removal},
    windows_lifecycle::connect_control,
    windows_process_identity::{PinnedProcess, ProcessIdentity, identity_for_handle},
};
use crate::windows_process::StartupChild;
use anyhow::{Context, Result, ensure};
use std::{
    ffi::OsString,
    fmt, fs, io,
    os::windows::io::{AsHandle, AsRawHandle},
    path::{Path, PathBuf},
    process::Command,
    sync::Arc,
    time::Duration,
};
use tokio::time::{Instant, sleep_until, timeout_at};
use windows_sys::Win32::{Foundation::CompareObjectHandles, System::Threading::GetProcessId};

const READINESS_BUDGET: Duration = Duration::from_secs(5);
const PROBE_BUDGET: Duration = Duration::from_millis(250);
const RETRY_INTERVAL: Duration = Duration::from_millis(25);
const CLEANUP_BUDGET: Duration = Duration::from_secs(5);

#[derive(Debug)]
pub struct UnavailableStartupExecutable {
    executable: PathBuf,
    source: io::Error,
}
impl fmt::Display for UnavailableStartupExecutable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "startup executable {} is no longer available; it may have been rebuilt, moved, or upgraded",
            self.executable.display()
        )
    }
}
impl std::error::Error for UnavailableStartupExecutable {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.source)
    }
}

/// Caller-owned configuration and environment must reconstruct the supplied
/// configured endpoint in the child. Inventory metadata never supplies those
/// roots. Main's native discovery wiring is a separate integration package.
pub struct HostStartup {
    pub executable: PathBuf,
    pub working_directory: Option<PathBuf>,
    pub config: Option<PathBuf>,
    pub targets: Vec<PathBuf>,
    pub env: Vec<(OsString, Option<OsString>)>,
    pub verbosity: u8,
    pub log: Option<PathBuf>,
}

impl HostStartup {
    pub fn new(executable: impl Into<PathBuf>) -> Self {
        Self {
            executable: executable.into(),
            working_directory: None,
            config: None,
            targets: Vec::new(),
            env: Vec::new(),
            verbosity: 0,
            log: None,
        }
    }

    fn command(&self, location: &EndpointLocation) -> Result<Command> {
        let launch_directory = std::env::current_dir()?;
        let absolute = |path: &Path| {
            if path.is_absolute() {
                path.to_owned()
            } else {
                launch_directory.join(path)
            }
        };
        let executable = absolute(&self.executable);
        match fs::metadata(&executable) {
            Err(source) if source.kind() == io::ErrorKind::NotFound => {
                return Err(UnavailableStartupExecutable { executable, source }.into());
            }
            value => {
                ensure!(value?.is_file(), "startup executable is not a file");
            }
        }
        let directory = self
            .working_directory
            .as_deref()
            .unwrap_or(location.project_root())
            .canonicalize()
            .context("cannot resolve host working directory")?;
        ensure!(
            directory.starts_with(location.project_root()),
            "host working directory is outside its configured project"
        );
        let directory = crate::windows_fs::ordinary_working_directory(&directory)?;
        let mut command = Command::new(executable);
        command
            .args(["--serve", "--detached-host", "--project-root"])
            .arg(location.project_root())
            .current_dir(directory);
        if let Some(config) = &self.config {
            command.arg("--config").arg(absolute(config));
        }
        for _ in 0..self.verbosity {
            command.arg("-v");
        }
        if let Some(log) = &self.log {
            command.arg("--log").arg(absolute(log));
        }
        command
            .arg("--")
            .args(self.targets.iter().map(|path| absolute(path)));
        for (name, value) in &self.env {
            if let Some(value) = value {
                command.env(name, value);
            } else {
                command.env_remove(name);
            }
        }
        Ok(command)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StartDisposition {
    Started,
    ExistingWinner,
}

/// Metadata is the actual authenticated ready publication, including when a
/// competing launcher won. The retained proof is not authority to terminate it.
#[derive(Debug)]
pub struct StartedHost {
    metadata: EndpointMetadata,
    peer: Arc<PinnedProcess>,
    disposition: StartDisposition,
}
impl StartedHost {
    pub fn metadata(&self) -> &EndpointMetadata {
        &self.metadata
    }
    pub fn peer(&self) -> &Arc<PinnedProcess> {
        &self.peer
    }
    pub fn disposition(&self) -> StartDisposition {
        self.disposition
    }
}

/// Readiness has one five-second budget. Cancellation keeps the provisional
/// child's job armed and requests whole-job termination on drop. Standard
/// handles are NUL; failures before logging starts have exit status, not captured
/// stderr. External job restrictions can refuse native detached creation.
pub async fn start_detached_host(
    location: &EndpointLocation,
    startup: HostStartup,
) -> Result<StartedHost> {
    let deadline = Instant::now() + READINESS_BUDGET;
    if retire_stale(location)? {
        return wait_existing(location, deadline).await;
    }
    let command = startup.command(location)?;
    ensure!(
        Instant::now() < deadline,
        "startup preparation exceeded its readiness budget"
    );
    let child = spawn_prepared(&command)?;
    let result = wait_ready(location, child, deadline).await;
    result.with_context(|| match startup.log {
        Some(log) => format!(
            "native host startup failed; selected diagnostic log: {}",
            log.display()
        ),
        None => "native host startup failed before authenticated readiness".to_owned(),
    })
}

fn spawn_prepared(command: &Command) -> Result<StartupChild> {
    match StartupChild::spawn(command) {
        Ok(child) => Ok(child),
        Err(source)
            if source.kind() == io::ErrorKind::NotFound
                && fs::metadata(command.get_program())
                    .is_err_and(|error| error.kind() == io::ErrorKind::NotFound) =>
        {
            Err(UnavailableStartupExecutable {
                executable: PathBuf::from(command.get_program()),
                source,
            }
            .into())
        }
        Err(error) => {
            Err(error).context("cannot create detached host under the current process/job policy")
        }
    }
}

/// True means some configured observation is still occupied or changed while
/// inspected. Missing readiness alone never authorizes a duplicate launch.
fn retire_stale(location: &EndpointLocation) -> Result<bool> {
    let mut observations = location.observe_registrations()?;
    if let Some(issue) = observations.issues.into_iter().next() {
        return Err(issue.error).context("configured startup registry observation failed");
    }
    if let Some(ready) = location.observe_ready()? {
        observations.candidates.push(ready);
    }
    let mut occupied = false;
    for candidate in observations.candidates {
        match candidate.inspect_process()? {
            Inspection::Present(_) => occupied = true,
            Inspection::Stale(evidence) => {
                if matches!(
                    evidence.remove_observed()?,
                    Removal::Changed | Removal::ProcessPresent
                ) {
                    occupied = true;
                }
            }
        }
    }
    Ok(occupied)
}

async fn wait_existing(location: &EndpointLocation, deadline: Instant) -> Result<StartedHost> {
    let mut detail = "configured publication is occupied without a ready record".to_owned();
    loop {
        let now = Instant::now();
        if now >= deadline {
            anyhow::bail!("existing publication could not be authenticated: {detail}");
        }
        match ready(location, deadline.min(now + PROBE_BUDGET)).await {
            Ok(Some((metadata, peer))) => return finish_winner(metadata, peer, deadline),
            Ok(None) => {}
            Err(error) => detail = bounded_detail(&error.to_string()),
        }
        sleep_until(deadline.min(Instant::now() + RETRY_INTERVAL)).await;
    }
}

async fn ready(
    location: &EndpointLocation,
    deadline: Instant,
) -> Result<Option<(EndpointMetadata, Arc<PinnedProcess>)>> {
    let Some(candidate) = location.observe_ready()? else {
        return Ok(None);
    };
    let metadata = candidate.metadata().clone();
    let client = timeout_at(deadline, connect_control(&metadata))
        .await
        .context("startup readiness probe timed out")??;
    Ok(Some((metadata, client.peer().clone())))
}

async fn wait_ready(
    location: &EndpointLocation,
    mut child: StartupChild,
    deadline: Instant,
) -> Result<StartedHost> {
    let identity = identity_for_handle(child.handle(), unsafe { GetProcessId(child.handle()) })?;
    let mut detail = "host has not published a ready record".to_owned();
    loop {
        let now = Instant::now();
        if now >= deadline {
            break;
        }
        match ready(location, deadline.min(now + PROBE_BUDGET)).await {
            Ok(Some((metadata, peer))) => {
                let same = unsafe {
                    CompareObjectHandles(child.handle(), peer.as_handle().as_raw_handle())
                } != 0;
                if same {
                    ensure!(
                        metadata.process == identity
                            && child.contains_process(peer.as_handle().as_raw_handle())?,
                        "ready host does not match provisional launch ownership"
                    );
                    ensure!(
                        Instant::now() < deadline,
                        "native host readiness deadline expired before handoff"
                    );
                    child.release()?;
                    // No await is permitted between disarming and returning.
                    return Ok(StartedHost {
                        metadata,
                        peer,
                        disposition: StartDisposition::Started,
                    });
                }
                ensure!(
                    !child.contains_process(peer.as_handle().as_raw_handle())?,
                    "a provisional child descendant cannot be treated as an external startup winner"
                );
                stop_provisional(&child).await?;
                return finish_winner(metadata, peer, deadline);
            }
            Ok(None) => {}
            Err(error) => {
                detail = bounded_detail(&error.to_string());
            }
        }
        // An exited loser can precede the winner's publication; continue the
        // same bounded observation instead of reporting that race as failure.
        sleep_until(deadline.min(Instant::now() + RETRY_INTERVAL)).await;
    }
    let status = child.exit_status()?;
    stop_provisional(&child).await?;
    // Only observations carrying this exact created identity are considered
    // here; another launcher's replacement and indeterminate records survive.
    cleanup_failed_launch(location, identity)?;
    anyhow::bail!(
        "host did not become ready within its startup budget (exit: {status:?}): {detail}"
    )
}

fn finish_winner(
    metadata: EndpointMetadata,
    peer: Arc<PinnedProcess>,
    deadline: Instant,
) -> Result<StartedHost> {
    ensure!(
        Instant::now() < deadline,
        "startup winner readiness deadline expired during loser cleanup"
    );
    ensure!(
        peer.is_alive()?,
        "authenticated startup winner exited during loser cleanup"
    );
    Ok(StartedHost {
        metadata,
        peer,
        disposition: StartDisposition::ExistingWinner,
    })
}

async fn stop_provisional(child: &StartupChild) -> Result<()> {
    child.terminate()?;
    let deadline = Instant::now() + CLEANUP_BUDGET;
    loop {
        // Job accounting can reach zero before the leader's process handle is
        // signaled. Stale-publication cleanup also requires that exact process
        // to have exited, so observe both boundaries before reporting success.
        if child.job_is_empty()? && child.exit_status()?.is_some() {
            return Ok(());
        }
        ensure!(
            Instant::now() < deadline,
            "provisional host job cleanup timed out"
        );
        sleep_until(deadline.min(Instant::now() + RETRY_INTERVAL)).await;
    }
}

fn cleanup_failed_launch(location: &EndpointLocation, identity: ProcessIdentity) -> Result<()> {
    let observations = location.observe_registrations()?;
    let mut failure = observations
        .issues
        .into_iter()
        .next()
        .map(|issue| anyhow::Error::from(issue.error));
    let mut candidates = observations.candidates;
    match location.observe_ready() {
        Ok(Some(candidate)) => candidates.push(candidate),
        Ok(None) => {}
        Err(error) => {
            failure.get_or_insert(error.into());
        }
    }
    for candidate in candidates {
        if candidate.metadata().process == identity
            && let Err(error) = remove_stopped(candidate)
        {
            failure.get_or_insert(error);
        }
    }
    if let Some(error) = failure {
        return Err(error);
    }
    Ok(())
}

fn remove_stopped(candidate: Candidate) -> Result<()> {
    if let Inspection::Stale(evidence) = candidate.inspect_process()? {
        evidence.remove_observed()?;
    }
    Ok(())
}

fn bounded_detail(value: &str) -> String {
    let mut end = value.len().min(1024);
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value[..end].to_owned()
}

#[cfg(test)]
mod tests;
