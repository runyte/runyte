// SPDX-License-Identifier: MPL-2.0

//! Starting and reaching persistent session hosts.
//!
//! Every caller that needs a host running at a known endpoint — restarting one,
//! serving a `--wait` request, or opening a workspace from the editor — goes
//! through [`start_detached_host`]. Keeping one recipe means the detachment,
//! environment, and readiness rules cannot drift apart between them.
//!
//! Nothing here owns a terminal or an editor. The executable to run is a
//! parameter rather than [`std::env::current_exe`] so tests can point it at a
//! test binary.

use std::{
    ffi::OsString,
    io::Read,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::Duration,
};

use anyhow::{Context, Result};

use crate::app::FrameGeometry;
use crate::protocol::{ClientRequest, HostResponse, validate_welcome};
use crate::{config, external_open, project_root};

use super::transport::{
    IncompatibleHost, LocalClient, LocalEndpoint, PublishedHost, RegisteredHost, process_is_alive,
    registered_hosts, request_process_exit,
};

/// How long to wait for a freshly started host to accept a connection, and how
/// often to retry while waiting.
const READINESS_ATTEMPTS: u32 = 200;
const READINESS_INTERVAL: Duration = Duration::from_millis(25);
/// A lifecycle control peer must complete its welcome or one request/response
/// exchange promptly. Interactive connections remain long-lived; this bound
/// applies only to the short control operations in this module.
#[cfg(not(test))]
const LIFECYCLE_IO_TIMEOUT: Duration = Duration::from_secs(2);
#[cfg(test)]
const LIFECYCLE_IO_TIMEOUT: Duration = Duration::from_millis(200);

/// Opens a non-interactive control connection and completes its handshake.
///
/// This doubles as the liveness check for a host: a successful return means the
/// host is accepting connections and speaks a compatible protocol.
pub async fn connect_control(endpoint: &LocalEndpoint) -> Result<LocalClient> {
    tokio::time::timeout(LIFECYCLE_IO_TIMEOUT, async {
        let mut client = LocalClient::connect(endpoint, FrameGeometry::default(), false).await?;
        match client.recv().await? {
            Some(response @ HostResponse::Welcome { .. }) => {
                validate_welcome(&response, false).map_err(anyhow::Error::msg)?;
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

/// Resolves a running host by full ID, unambiguous ID prefix, exact name, or
/// project directory.
pub fn resolve_registered_host(selector: &Path) -> Result<RegisteredHost> {
    let working_directory = std::env::current_dir().ok();
    resolve_registered_host_from(selector, working_directory.as_deref(), registered_hosts()?)
}

/// Resolves a running host while interpreting relative directory selectors
/// from `working_directory`. Exact names and IDs still compare against the
/// selector itself.
pub fn resolve_registered_host_from_directory(
    selector: &Path,
    working_directory: &Path,
) -> Result<RegisteredHost> {
    resolve_registered_host_from(selector, Some(working_directory), registered_hosts()?)
}

pub(super) fn resolve_registered_host_from(
    selector: &Path,
    working_directory: Option<&Path>,
    hosts: Vec<RegisteredHost>,
) -> Result<RegisteredHost> {
    let text = selector.to_str();
    let lower_id = text.map(str::to_ascii_lowercase);
    let id_selector = lower_id
        .as_deref()
        .filter(|value| !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_hexdigit()));
    let supplied_directory = if selector.is_absolute() {
        Some(selector.to_path_buf())
    } else {
        working_directory.map(|current| current.join(selector))
    };
    let directory =
        supplied_directory.map(|directory| directory.canonicalize().unwrap_or(directory));
    let mut exact = hosts
        .iter()
        .filter(|host| {
            lower_id
                .as_deref()
                .is_some_and(|id| id.len() == host.id.len() && host.id == id)
                || text.is_some_and(|name| host.name.as_deref() == Some(name))
                || directory
                    .as_ref()
                    .is_some_and(|directory| host.project_root == *directory)
        })
        .cloned()
        .collect::<Vec<_>>();
    exact.dedup_by(|left, right| left.id == right.id);
    if exact.len() == 1 {
        return Ok(exact.pop().expect("one exact host matched"));
    }
    if exact.len() > 1 {
        anyhow::bail!(
            "workspace selector {} is ambiguous; use the displayed workspace ID or directory",
            selector.display()
        );
    }

    let mut prefixes = hosts
        .into_iter()
        .filter(|host| id_selector.is_some_and(|id| host.id.starts_with(id)))
        .collect::<Vec<_>>();
    prefixes.dedup_by(|left, right| left.id == right.id);
    match prefixes.len() {
        0 => anyhow::bail!(
            "no running session matches {}; use --session-list to see available sessions",
            selector.display()
        ),
        1 => Ok(prefixes.pop().expect("one host matched")),
        _ => anyhow::bail!(
            "workspace selector {} is ambiguous; use the displayed workspace ID or directory",
            selector.display()
        ),
    }
}

/// Changes a running host's persistent display name.
pub async fn rename_host(endpoint: &LocalEndpoint, name: &str) -> Result<()> {
    let mut client = connect_control(endpoint).await?;
    tokio::time::timeout(LIFECYCLE_IO_TIMEOUT, async {
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

/// Asks a running host to stop. The host owns the dirty-buffer refusal.
pub async fn shutdown_host(endpoint: &LocalEndpoint) -> Result<()> {
    let mut client = connect_control(endpoint).await?;
    shutdown_request(&mut client, ClientRequest::Shutdown, "shutdown").await
}

/// Explicitly stops a host even when doing so abandons protected live state.
pub async fn force_shutdown_host(endpoint: &LocalEndpoint) -> Result<()> {
    let mut client = connect_control(endpoint).await?;
    shutdown_request(&mut client, ClientRequest::ForceShutdown, "force-shutdown").await
}

async fn shutdown_request(
    client: &mut LocalClient,
    request: ClientRequest,
    description: &str,
) -> Result<()> {
    tokio::time::timeout(LIFECYCLE_IO_TIMEOUT, async {
        client.send(&request).await?;
        match client.recv().await? {
            Some(HostResponse::ShuttingDown) | None => Ok(()),
            Some(HostResponse::Refused { message } | HostResponse::Error { message }) => {
                anyhow::bail!(message)
            }
            Some(response) => anyhow::bail!("unexpected {description} response: {response:?}"),
        }
    })
    .await
    .with_context(|| format!("workspace host {description} request timed out"))?
}

/// Stops a host whose protocol this build cannot speak, then clears the
/// endpoint it leaves behind.
///
/// [`shutdown_host`] is always the right call first: a host that can be asked
/// to stop owns the answer, and refuses while it holds unsaved buffers. Nothing
/// can ask a host of another version that question — the request would not
/// parse — so the choice is between stopping its process and leaving the
/// workspace permanently unreachable. This refuses a host that could have been
/// asked, so the abrupt path can never stand in for the protocol one.
pub async fn terminate_incompatible_host(endpoint: &LocalEndpoint) -> Result<PublishedHost> {
    let host = endpoint
        .published_host()?
        .context("no workspace host is running there")?;
    anyhow::ensure!(
        !host.speaks_current_protocol(),
        "this persistent session speaks the current protocol; stop it with --session-stop so its unsaved buffers are respected"
    );
    request_process_exit(host.pid)?;
    for _ in 0..READINESS_ATTEMPTS {
        if !process_is_alive(host.pid)? {
            endpoint.clear_published_host(host.pid)?;
            return Ok(host);
        }
        tokio::time::sleep(READINESS_INTERVAL).await;
    }
    anyhow::bail!(
        "workspace host process {} did not stop; end it and run this again",
        host.pid
    )
}

/// Stops a running host and starts a detached replacement at the same endpoint.
pub async fn restart_host(endpoint: &LocalEndpoint, startup: HostStartup) -> Result<()> {
    shutdown_host(endpoint).await?;
    for _ in 0..READINESS_ATTEMPTS {
        if !endpoint.metadata().exists() && !endpoint.socket().exists() {
            return start_detached_host(endpoint, startup).await;
        }
        tokio::time::sleep(READINESS_INTERVAL).await;
    }
    anyhow::bail!("workspace host did not finish shutting down")
}

/// Replaces a host after an explicit acknowledgement that protected state is
/// discarded.
pub async fn force_restart_host(endpoint: &LocalEndpoint, startup: HostStartup) -> Result<()> {
    force_shutdown_host(endpoint).await?;
    for _ in 0..READINESS_ATTEMPTS {
        if !endpoint.metadata().exists() && !endpoint.socket().exists() {
            return start_detached_host(endpoint, startup).await;
        }
        tokio::time::sleep(READINESS_INTERVAL).await;
    }
    anyhow::bail!("workspace host did not finish shutting down")
}

/// Resolves a directory to the endpoint for the workspace discovered there.
/// This is shared by the attached-client switch loop and background starts so
/// both enforce the same project and state-root safety rules.
pub fn resolve_workspace_endpoint(
    requested: &Path,
    state: &Path,
    config_path: Option<&Path>,
) -> Result<LocalEndpoint> {
    resolve_workspace_endpoint_with_runtime(requested, state, config_path, None)
}

pub(super) fn resolve_workspace_endpoint_with_runtime(
    requested: &Path,
    state: &Path,
    config_path: Option<&Path>,
    runtime: Option<&Path>,
) -> Result<LocalEndpoint> {
    anyhow::ensure!(
        requested.is_dir(),
        "workspace is unavailable: {}",
        requested.display()
    );
    let requested = requested
        .canonicalize()
        .with_context(|| format!("cannot resolve workspace {}", requested.display()))?;
    let project_root = project_root::discover(&requested, state)?.context(
        "no Git repository or Runyte state directory was found there; open it once outside the editor to choose where its data lives",
    )?;
    let state_root = project_root::resolve_state_root(&project_root, state);
    let mut reserved_user_roots = config_path
        .map(|path| config::config_root_for(path, &project_root))
        .into_iter()
        .collect::<Vec<_>>();
    if let Some(cache_root) = external_open::cache_root() {
        reserved_user_roots.push(cache_root);
    }
    project_root::validate_state_root(&state_root, &reserved_user_roots)?;
    match runtime {
        Some(runtime) => {
            LocalEndpoint::discover_with_runtime(&state_root, &project_root, Some(runtime))
        }
        None => LocalEndpoint::discover(&state_root, &project_root),
    }
}

/// Reaches a workspace, starting its detached host when necessary.
pub async fn ensure_workspace_host(
    requested: &Path,
    state: &Path,
    config_path: Option<&Path>,
    startup: HostStartup,
) -> Result<LocalEndpoint> {
    let endpoint = resolve_workspace_endpoint(requested, state, config_path)?;
    if let Err(error) = connect_control(&endpoint).await {
        // A host of another version still owns this endpoint, so no
        // replacement could bind here anyway. The error names the process and
        // how to stop it rather than reporting a failure to start.
        if error.downcast_ref::<IncompatibleHost>().is_some() {
            return Err(error);
        }
        start_detached_host(&endpoint, startup).await?;
    }
    Ok(endpoint)
}

/// What a new host process needs beyond its endpoint.
#[derive(Clone, Debug)]
pub struct HostStartup {
    /// The Runyte executable to run. A parameter so tests can inject theirs.
    pub executable: PathBuf,
    /// Directory the editor should begin in. This may be below the endpoint's
    /// project root, but must still resolve to that project. Callers which are
    /// not continuing an editor invocation leave this unset and start at the
    /// project root.
    pub working_directory: Option<PathBuf>,
    /// Configuration to hand the host. Resolved to an absolute path before the
    /// child is spawned, because the child may run in another directory.
    pub config: Option<PathBuf>,
    /// Files the host should open, used by `--wait`.
    pub targets: Vec<PathBuf>,
    /// Environment overrides for the child, beyond the runtime directory this
    /// module derives from the endpoint. Tests inject their private
    /// `XDG_CACHE_HOME` here so a spawned host cannot register itself in the
    /// person's real cache, the same reason
    /// [`LocalEndpoint::discover_with_runtime`] takes an explicit runtime root.
    pub env: Vec<(OsString, OsString)>,
    /// What the failure message calls this host, for example `"restarted"`.
    pub description: &'static str,
    /// How many `-v` occurrences the host should start with. Verbosity is a
    /// property of host startup: an attachment never reconfigures a running
    /// host's logger.
    pub verbosity: u8,
    /// An explicit diagnostic log destination for the host. Resolved to an
    /// absolute path before the child is spawned, because the child may run in
    /// another directory.
    pub log: Option<PathBuf>,
}

impl HostStartup {
    pub fn new(executable: impl Into<PathBuf>, description: &'static str) -> Self {
        Self {
            executable: executable.into(),
            working_directory: None,
            config: None,
            targets: Vec::new(),
            env: Vec::new(),
            description,
            verbosity: 0,
            log: None,
        }
    }

    /// Hands this process's selected logging to the host it starts.
    pub fn with_logging(mut self, verbosity: u8, log: Option<&Path>) -> Self {
        self.verbosity = verbosity;
        self.log = log.map(absolute);
        self
    }

    pub fn with_env(mut self, key: impl Into<OsString>, value: impl Into<OsString>) -> Self {
        self.env.push((key.into(), value.into()));
        self
    }

    pub fn with_working_directory(mut self, directory: impl Into<PathBuf>) -> Self {
        self.working_directory = Some(directory.into());
        self
    }

    pub fn with_config(mut self, config: Option<&Path>) -> Self {
        // The child's working directory can differ from the caller's, so a
        // relative path given on the invoking command line needs resolving
        // before the child starts.
        self.config = config.map(absolute);
        self
    }

    pub fn with_targets(mut self, targets: Vec<PathBuf>) -> Self {
        self.targets = targets;
        self
    }
}

/// Resolves a path the spawned child will read from another directory.
fn absolute(path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map(|directory| directory.join(path))
            .unwrap_or_else(|_| path.to_path_buf())
    }
}

/// Starts a host for `endpoint` and returns once it accepts connections.
///
/// The child is detached with `setsid` so it outlives the caller's controlling
/// terminal, and it is reaped on a thread of its own: a long-lived editor that
/// starts hosts must not leave zombies behind, because `kill(pid, 0)` reports a
/// zombie as alive and a stale registration would then block the next start.
pub async fn start_detached_host(endpoint: &LocalEndpoint, startup: HostStartup) -> Result<()> {
    let project_root = endpoint.project_root().to_path_buf();
    let working_directory = startup
        .working_directory
        .as_deref()
        .unwrap_or(&project_root)
        .canonicalize()
        .with_context(|| {
            format!(
                "cannot resolve workspace host working directory {}",
                startup
                    .working_directory
                    .as_deref()
                    .unwrap_or(&project_root)
                    .display()
            )
        })?;
    anyhow::ensure!(
        working_directory.starts_with(&project_root),
        "workspace host working directory {} is outside project root {}",
        working_directory.display(),
        project_root.display()
    );
    let mut command = Command::new(&startup.executable);
    command
        .arg("--serve")
        // The endpoint already names the workspace this host is being started
        // for, so the child is told it rather than left to rediscover it. Its
        // stdin is null, so a project whose root cannot be derived from the
        // filesystem alone — no Git root and no state directory yet — would
        // otherwise reach an unanswerable prompt and exit.
        .arg("--project-root")
        .arg(&project_root)
        .current_dir(&working_directory)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    if let Some(runtime_root) = endpoint.runtime_root() {
        command.env("XDG_RUNTIME_DIR", runtime_root);
    } else {
        // Endpoint discovery is environment-dependent. A host chosen at a
        // fallback endpoint must start at that same fallback endpoint even when
        // this process happens to have a usable XDG runtime directory.
        command.env_remove("XDG_RUNTIME_DIR");
    }
    for (key, value) in &startup.env {
        command.env(key, value);
    }
    if let Some(config) = startup.config.as_deref() {
        command.arg("--config").arg(config);
    }
    for _ in 0..startup.verbosity {
        command.arg("-v");
    }
    if let Some(log) = startup.log.as_deref() {
        command.arg("--log").arg(log);
    }
    command.args(&startup.targets);

    use std::os::unix::process::CommandExt;
    // SAFETY: `setsid` is async-signal-safe and runs in the child immediately
    // before exec. The host must outlive the caller's controlling terminal and
    // its eventual hangup.
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }

    let child = command.spawn().with_context(|| {
        format!(
            "cannot start {} workspace host for {}",
            startup.description,
            project_root.display()
        )
    })?;
    let mut child = ReapedChild::new(child);

    for _ in 0..READINESS_ATTEMPTS {
        // Connect first. Two clients may race the same workspace, and the
        // loser's child exits immediately because the winner already holds the
        // endpoint's identity lock. Checking the child before connecting would
        // report that race as a failure even though a host is up.
        if connect_control(endpoint).await.is_ok() {
            return Ok(());
        }
        if let Some(status) = child.exited()? {
            // One last attempt: the child may have exited because another
            // process won the race and is now serving this endpoint.
            if connect_control(endpoint).await.is_ok() {
                return Ok(());
            }
            let detail = child.stderr_detail();
            let description = startup.description;
            if detail.is_empty() {
                anyhow::bail!("{description} workspace host exited with {status}");
            }
            anyhow::bail!("{description} workspace host exited with {status}: {detail}");
        }
        tokio::time::sleep(READINESS_INTERVAL).await;
    }
    child.kill();
    anyhow::bail!(
        "{} workspace host did not publish its endpoint",
        startup.description
    )
}

/// A spawned host that is waited for on a thread rather than left as a zombie.
///
/// Reaping matters beyond tidiness: [`super::transport`] treats a process as
/// alive whenever `kill(pid, 0)` succeeds, which it does for a zombie, so an
/// unreaped dead host would keep its registration looking live and refuse the
/// next start for that workspace.
struct ReapedChild(Option<Child>);

impl ReapedChild {
    fn new(child: Child) -> Self {
        Self(Some(child))
    }

    fn exited(&mut self) -> Result<Option<std::process::ExitStatus>> {
        let Some(child) = self.0.as_mut() else {
            return Ok(None);
        };
        Ok(child.try_wait()?)
    }

    fn stderr_detail(&mut self) -> String {
        let mut detail = String::new();
        if let Some(child) = self.0.as_mut()
            && let Some(stderr) = child.stderr.as_mut()
        {
            let _ = stderr.read_to_string(&mut detail);
        }
        detail.trim().to_owned()
    }

    fn kill(&mut self) {
        if let Some(child) = self.0.as_mut() {
            let _ = child.kill();
        }
    }
}

impl Drop for ReapedChild {
    fn drop(&mut self) {
        if let Some(mut child) = self.0.take() {
            // Nothing needs the exit status; the point is to keep a dead host
            // from lingering in the process table of a long-lived editor.
            std::thread::spawn(move || {
                let _ = child.wait();
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        fs, io,
        sync::atomic::{AtomicU64, Ordering},
    };

    use super::*;
    use crate::{
        protocol::FeatureGroup,
        workspace::transport::{LocalServer, PROTOCOL_VERSION, ServerEvent},
    };

    fn endpoint(label: &str) -> (PathBuf, LocalEndpoint) {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let root = Path::new("/tmp").join(format!(
            "ryt-lifecycle-{label}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&root).unwrap();
        let endpoint = LocalEndpoint::new(&root.join(".runyte"), &root).unwrap();
        (root, endpoint)
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
            Err(error) => panic!("cannot bind lifecycle test transport: {error:#}"),
        }
    }

    async fn welcome(responses: &super::super::transport::ResponseSender) {
        responses
            .send(HostResponse::Welcome {
                protocol: PROTOCOL_VERSION,
                pid: std::process::id(),
                features: vec![
                    FeatureGroup::Control,
                    FeatureGroup::Buffers,
                    FeatureGroup::Wait,
                ],
                host_version: env!("CARGO_PKG_VERSION").to_owned(),
            })
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn control_handshake_has_a_deadline() {
        let (root, endpoint) = endpoint("handshake-timeout");
        let Some(mut server) = bind_or_skip(&endpoint).await else {
            fs::remove_dir_all(root).unwrap();
            return;
        };
        let target = endpoint.clone();
        let connection = tokio::spawn(async move { connect_control(&target).await });
        let Some(ServerEvent::Connected { responses, .. }) = server.recv().await else {
            panic!("control client did not connect")
        };

        let error = match connection.await.unwrap() {
            Ok(_) => panic!("silent host unexpectedly completed its handshake"),
            Err(error) => error.to_string(),
        };
        assert!(error.contains("handshake timed out"), "{error}");

        drop(responses);
        drop(server);
        endpoint.cleanup().unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn lifecycle_request_response_has_a_deadline() {
        let (root, endpoint) = endpoint("response-timeout");
        let Some(mut server) = bind_or_skip(&endpoint).await else {
            fs::remove_dir_all(root).unwrap();
            return;
        };
        let target = endpoint.clone();
        let rename = tokio::spawn(async move { rename_host(&target, "renamed").await });
        let Some(ServerEvent::Connected { responses, .. }) = server.recv().await else {
            panic!("control client did not connect")
        };
        welcome(&responses).await;
        assert!(matches!(
            server.recv().await,
            Some(ServerEvent::Request {
                request: ClientRequest::RenameHost { .. },
                ..
            })
        ));

        let error = rename.await.unwrap().unwrap_err().to_string();
        assert!(error.contains("rename request timed out"), "{error}");

        drop(server);
        endpoint.cleanup().unwrap();
        fs::remove_dir_all(root).unwrap();
    }
}
