// SPDX-License-Identifier: MPL-2.0

//! Provisional native host ownership and authenticated detached handoff.
//! Public attachment and private handoffs use this lifecycle off the editor loop:
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
    fmt, fs,
    future::Future,
    io,
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
    test_harness_helper: Option<String>,
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
            test_harness_helper: None,
        }
    }

    #[doc(hidden)]
    pub fn with_test_harness_helper(mut self, helper: impl Into<String>) -> Self {
        self.test_harness_helper = Some(helper.into());
        self
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
        if let Some(helper) = &self.test_harness_helper {
            command.args([
                "--exact",
                helper,
                "--ignored",
                "--nocapture",
                "--test-threads=1",
            ]);
        } else {
            command
                .args(["--serve", "--detached-host", "--project-root"])
                .arg(location.project_root());
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
        }
        command.current_dir(directory);
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

struct ProvisionalHost {
    child: StartupChild,
    location: EndpointLocation,
    identity: ProcessIdentity,
}

/// An authenticated startup result whose newly-created process remains in its
/// kill-on-close job until the caller explicitly accepts it. Existing winners
/// carry no provisional ownership and are never terminated by cancellation.
pub(crate) struct PreparedHost {
    metadata: EndpointMetadata,
    peer: Arc<PinnedProcess>,
    disposition: StartDisposition,
    provisional: Option<ProvisionalHost>,
}

pub(crate) enum PreparedAcceptance {
    Accepted,
    Refused {
        error: anyhow::Error,
        cleanup: Result<()>,
    },
}

impl PreparedHost {
    pub(crate) fn metadata(&self) -> &EndpointMetadata {
        &self.metadata
    }

    pub(crate) fn peer(&self) -> &Arc<PinnedProcess> {
        &self.peer
    }

    pub(crate) fn disposition(&self) -> StartDisposition {
        self.disposition
    }

    pub(crate) fn accept(mut self) -> Result<StartedHost> {
        if let Some(provisional) = self.provisional.as_mut() {
            provisional.child.release()?;
        }
        self.provisional = None;
        Ok(StartedHost {
            metadata: self.metadata,
            peer: self.peer,
            disposition: self.disposition,
        })
    }

    /// Releases a prepared host, but keeps failed release ownership long
    /// enough to terminate the complete provisional job and remove only that
    /// launch's publication before returning the error.
    pub(crate) async fn accept_or_settle(mut self) -> PreparedAcceptance {
        if let Some(provisional) = self.provisional.as_mut()
            && let Err(error) = provisional.child.release()
        {
            let error = anyhow::Error::from(error).context("cannot release prepared native host");
            let cleanup = self
                .settle()
                .await
                .context("failed native host release cleanup did not settle");
            return PreparedAcceptance::Refused { error, cleanup };
        }
        self.provisional = None;
        PreparedAcceptance::Accepted
    }

    pub(crate) async fn settle(mut self) -> Result<()> {
        let Some(provisional) = self.provisional.take() else {
            return Ok(());
        };
        stop_provisional(&provisional.child).await?;
        cleanup_failed_launch(&provisional.location, provisional.identity)
    }
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
    prepare_detached_host(location, startup).await?.accept()
}

/// Exercises the same detached child/job creation policy before a restart
/// retires its selected host. `--version` exits before workspace or runtime
/// initialization. Successful return proves the probe process has exited and
/// its private job is empty; later startup can still fail independently.
pub async fn preflight_detached_host(
    location: &EndpointLocation,
    startup: &HostStartup,
) -> Result<()> {
    let planned = startup.command(location)?;
    let mut probe = Command::new(planned.get_program());
    probe.arg("--version");
    if let Some(directory) = planned.get_current_dir() {
        probe.current_dir(directory);
    }
    for (key, value) in planned.get_envs() {
        if let Some(value) = value {
            probe.env(key, value);
        } else {
            probe.env_remove(key);
        }
    }
    let child = StartupChild::spawn(&probe)
        .context("detached host preflight was denied; selected session was not stopped")?;
    let deadline = Instant::now() + READINESS_BUDGET;
    let outcome = loop {
        match child.exit_status() {
            Ok(Some(status)) => {
                break if status.success() {
                    Ok(())
                } else {
                    Err(anyhow::anyhow!(
                        "detached host preflight exited with {status}"
                    ))
                };
            }
            Ok(None) => {}
            Err(error) => break Err(error.into()),
        }
        if Instant::now() >= deadline {
            break Err(anyhow::anyhow!(
                "detached host preflight did not exit in time"
            ));
        }
        sleep_until(deadline.min(Instant::now() + RETRY_INTERVAL)).await;
    };
    let cleanup = stop_provisional(&child)
        .await
        .context("detached host preflight job did not settle; selected session was not stopped");
    cleanup?;
    outcome
}

/// Starts a public attachment host while retaining the provisional job through
/// launch cancellation. A cancelled launch settles only its own new process;
/// an existing authenticated winner remains untouched.
pub async fn start_detached_host_cancellable<C>(
    location: &EndpointLocation,
    startup: HostStartup,
    cancellation: C,
) -> Result<StartedHost>
where
    C: Future<Output = &'static str>,
{
    tokio::pin!(cancellation);
    let prepared = prepare_detached_host_cancellable(location, startup, &mut cancellation).await?;
    tokio::select! {
        biased;
        reason = &mut cancellation => {
            prepared.settle().await?;
            anyhow::bail!(reason);
        }
        _ = std::future::ready(()) => {}
    }
    let metadata = prepared.metadata().clone();
    let peer = Arc::clone(prepared.peer());
    let disposition = prepared.disposition();
    match prepared.accept_or_settle().await {
        PreparedAcceptance::Accepted => Ok(StartedHost {
            metadata,
            peer,
            disposition,
        }),
        PreparedAcceptance::Refused { error, cleanup } => {
            cleanup.context("native host startup cleanup failed")?;
            Err(error)
        }
    }
}

/// Authenticates readiness without releasing a newly-created host from its
/// provisional job. This is used by workflows whose own authority must remain
/// valid through a later commit boundary.
pub(crate) async fn prepare_detached_host(
    location: &EndpointLocation,
    startup: HostStartup,
) -> Result<PreparedHost> {
    prepare_detached_host_cancellable(location, startup, std::future::pending::<&'static str>())
        .await
}

/// The service-owned ParentAttach variant interrupts readiness as soon as its
/// caller loses authority. The still-armed child remains here while its one
/// cleanup budget proves job emptiness and removes its exact publication.
pub(crate) async fn prepare_detached_host_cancellable<C>(
    location: &EndpointLocation,
    startup: HostStartup,
    cancellation: C,
) -> Result<PreparedHost>
where
    C: Future<Output = &'static str>,
{
    let deadline = Instant::now() + READINESS_BUDGET;
    if retire_stale(location)? {
        tokio::pin!(cancellation);
        return tokio::select! {
            biased;
            reason = &mut cancellation => anyhow::bail!(reason),
            result = wait_existing_prepared(location, deadline) => result,
        };
    }
    let command = startup.command(location)?;
    ensure!(
        Instant::now() < deadline,
        "startup preparation exceeded its readiness budget"
    );
    let child = spawn_prepared(&command)?;
    let identity = identity_for_handle(child.handle(), unsafe { GetProcessId(child.handle()) })?;
    tokio::pin!(cancellation);
    let observed = tokio::select! {
        biased;
        reason = &mut cancellation => {
            stop_provisional(&child).await?;
            cleanup_failed_launch(location, identity)?;
            anyhow::bail!(reason);
        }
        result = observe_ready_prepared(location, &child, identity, deadline) => result,
    };
    finish_prepared_observation(location, child, identity, deadline, observed)
        .await
        .with_context(|| match startup.log {
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

async fn wait_existing_prepared(
    location: &EndpointLocation,
    deadline: Instant,
) -> Result<PreparedHost> {
    let mut detail = "configured publication is occupied without a ready record".to_owned();
    loop {
        let now = Instant::now();
        if now >= deadline {
            anyhow::bail!("existing publication could not be authenticated: {detail}");
        }
        match ready(location, deadline.min(now + PROBE_BUDGET)).await {
            Ok(Some((metadata, peer))) => return finish_winner_prepared(metadata, peer, deadline),
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

#[cfg(test)]
async fn wait_ready(
    location: &EndpointLocation,
    child: StartupChild,
    deadline: Instant,
) -> Result<StartedHost> {
    wait_ready_prepared(location, child, deadline)
        .await?
        .accept()
}

#[cfg(test)]
async fn wait_ready_prepared(
    location: &EndpointLocation,
    child: StartupChild,
    deadline: Instant,
) -> Result<PreparedHost> {
    let identity = identity_for_handle(child.handle(), unsafe { GetProcessId(child.handle()) })?;
    let observed = observe_ready_prepared(location, &child, identity, deadline).await;
    finish_prepared_observation(location, child, identity, deadline, observed).await
}

enum PreparedObservation {
    Created(EndpointMetadata, Arc<PinnedProcess>),
    Winner(EndpointMetadata, Arc<PinnedProcess>),
}

async fn observe_ready_prepared(
    location: &EndpointLocation,
    child: &StartupChild,
    identity: ProcessIdentity,
    deadline: Instant,
) -> Result<PreparedObservation> {
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
                    return Ok(PreparedObservation::Created(metadata, peer));
                }
                ensure!(
                    !child.contains_process(peer.as_handle().as_raw_handle())?,
                    "a provisional child descendant cannot be treated as an external startup winner"
                );
                return Ok(PreparedObservation::Winner(metadata, peer));
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
    anyhow::bail!(
        "host did not become ready within its startup budget (exit: {status:?}): {detail}"
    )
}

async fn finish_prepared_observation(
    location: &EndpointLocation,
    child: StartupChild,
    identity: ProcessIdentity,
    deadline: Instant,
    observed: Result<PreparedObservation>,
) -> Result<PreparedHost> {
    match observed {
        Ok(PreparedObservation::Created(metadata, peer)) => Ok(PreparedHost {
            metadata,
            peer,
            disposition: StartDisposition::Started,
            provisional: Some(ProvisionalHost {
                child,
                location: location.clone(),
                identity,
            }),
        }),
        Ok(PreparedObservation::Winner(metadata, peer)) => {
            stop_provisional(&child).await?;
            cleanup_failed_launch(location, identity)?;
            finish_winner_prepared(metadata, peer, deadline)
        }
        Err(error) => {
            stop_provisional(&child).await?;
            cleanup_failed_launch(location, identity)?;
            Err(error)
        }
    }
}

#[cfg(test)]
fn finish_winner(
    metadata: EndpointMetadata,
    peer: Arc<PinnedProcess>,
    deadline: Instant,
) -> Result<StartedHost> {
    finish_winner_prepared(metadata, peer, deadline)?.accept()
}

fn finish_winner_prepared(
    metadata: EndpointMetadata,
    peer: Arc<PinnedProcess>,
    deadline: Instant,
) -> Result<PreparedHost> {
    ensure!(
        Instant::now() < deadline,
        "startup winner readiness deadline expired during loser cleanup"
    );
    ensure!(
        peer.is_alive()?,
        "authenticated startup winner exited during loser cleanup"
    );
    Ok(PreparedHost {
        metadata,
        peer,
        disposition: StartDisposition::ExistingWinner,
        provisional: None,
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
