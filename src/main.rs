// SPDX-License-Identifier: MPL-2.0

use std::{
    fs,
    io::{self, Write, stdout},
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
#[cfg(unix)]
use crossterm::event::{
    KeyboardEnhancementFlags, PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
};
use crossterm::{
    ExecutableCommand,
    cursor::Show,
    event::{
        DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
        Event as CrosstermEvent, EventStream, KeyEventKind,
    },
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use futures_util::StreamExt;
use ratatui::{Terminal, backend::CrosstermBackend};
use runyte::{
    app::{App, PersistentExitRequest},
    command::{CommandCategory, CommandExecutionContext, CommandInvocation, EditorCommand},
    config::{self, Config, WorkspaceMode},
    external_open, file_monitor, file_picker,
    git::{GitCliProvider, GitService, GitServiceEvent},
    input::{InputEvent, KeyStroke, PointerEvent, PointerEventKind},
    key_hints::{HintEventResult, KeyHintState},
    keymap::{BindingTarget, KeySequence, Lookup},
    launch::{LaunchArguments, LaunchMode, LaunchTarget},
    log::{self as diagnostic_log, Level as LogLevel, Role as LogRole},
    log_debug, log_error, log_info, log_trace, log_warn,
    lsp::{self, LspCommand, LspEvent, LspHandle},
    notification::{NotificationDraft, NotificationSeverity},
    project_root,
    startup::{StartupPhase, StartupTrace},
    syntax::{self, SyntaxEvents},
    terminal::{self, TerminalEvents},
    tui::input::convert_event,
    ui, word_index,
    workspace::{HostCommand, HostEvent, HostInputOutcome, WorkspaceHost, workspace_id},
};

const STATUS_ANIMATION_INTERVAL: Duration = Duration::from_millis(80);
const TERMINAL_FRAME_INTERVAL: Duration = Duration::from_millis(16);
/// How long a shutting-down host waits for its connections to finish writing.
/// Longer than the transport's own write stall budget, so a peer that is
/// merely slow is flushed and only one that has stopped reading is cut off.
#[cfg(unix)]
const SHUTDOWN_FLUSH_BUDGET: Duration = Duration::from_secs(3);

#[cfg(unix)]
use runyte::protocol::{MAX_POINTER_REPETITIONS, WaitStatus, WaitToken, validate_welcome};
#[cfg(unix)]
use runyte::workspace::lifecycle::{
    HostStartup, connect_control, ensure_workspace_host, force_restart_host, force_shutdown_host,
    resolve_registered_host, resolve_registered_host_from_directory, resolve_workspace_endpoint,
    restart_host, shutdown_host, start_detached_host, terminate_incompatible_host,
};
#[cfg(unix)]
use runyte::workspace::transport::{
    ClientRequest, FeatureGroup, HostResponse, IncompatibleHost, LocalClient, LocalEndpoint,
    LocalServer, ServerEvent, TransportChange, decode_path, encode_path,
    registered_hosts_all_namespaces,
};
#[cfg(unix)]
use runyte::workspace::{
    WorkspaceService, abbreviated_id_width, clear_stopped_sessions, ensure_recent_workspace,
    known_workspaces, known_workspaces_all_namespaces, record_recent_workspace,
    record_workspace_activity, rename_known_workspace, resolve_known_workspace,
    resolve_known_workspace_from_directory,
};

fn main() -> Result<()> {
    let mut startup = StartupTrace::new();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("failed to start the async runtime")?;
    let result = runtime.block_on(run(&mut startup));
    drop(runtime);
    // Only that the process is ending, never the chain that ended it. An
    // arbitrary propagated error is unclassified text — an option's value, a
    // path taken from the environment, whatever a future layer attaches — and
    // this is a durable file. The boundaries that know what a failure means
    // record it themselves, and a startup failure still reaches stderr, which
    // is where a launcher reads a detached session's exit.
    match &result {
        Ok(()) => log_info!("process", "runyte exited"),
        Err(_) => log_error!("process", "runyte exited with an error"),
    }
    runyte::log::shutdown();
    #[cfg(unix)]
    if let Err(error) = &result
        && let Some(signal) = error.downcast_ref::<TerminatedBySignal>()
    {
        std::process::exit(128 + signal.0);
    }
    result
}

#[derive(Debug)]
struct TerminatedBySignal(i32);

impl std::fmt::Display for TerminatedBySignal {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "terminated by signal {}", self.0)
    }
}

impl std::error::Error for TerminatedBySignal {}

#[cfg(unix)]
struct TerminationSignals {
    poll: tokio::time::Interval,
}

#[cfg(unix)]
impl TerminationSignals {
    fn new() -> Result<Self> {
        install_termination_handlers()?;
        let mut poll = tokio::time::interval(Duration::from_millis(25));
        poll.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        Ok(Self { poll })
    }

    async fn recv(&mut self) -> i32 {
        loop {
            if let Some(signal) = self.received() {
                return signal;
            }
            self.poll.tick().await;
        }
    }

    fn received(&self) -> Option<i32> {
        let signal = RECEIVED_TERMINATION.swap(0, std::sync::atomic::Ordering::Relaxed);
        (signal != 0).then_some(signal)
    }
}

#[cfg(unix)]
static RECEIVED_TERMINATION: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);

#[cfg(unix)]
extern "C" fn record_termination(signal: libc::c_int) {
    RECEIVED_TERMINATION.store(signal, std::sync::atomic::Ordering::Relaxed);
}

#[cfg(unix)]
fn install_termination_handlers() -> Result<()> {
    static INSTALLED: std::sync::OnceLock<std::result::Result<(), i32>> =
        std::sync::OnceLock::new();
    let result = INSTALLED.get_or_init(|| {
        // SAFETY: a zeroed sigaction is a valid starting point; the handler
        // only performs an atomic store, which is async-signal-safe on the
        // supported lock-free Unix targets. Each call supplies a live action
        // and does not request the previous disposition.
        let mut action: libc::sigaction = unsafe { std::mem::zeroed() };
        action.sa_sigaction = record_termination as *const () as usize;
        action.sa_flags = 0;
        // SAFETY: `sa_mask` is owned writable storage in `action`.
        if unsafe { libc::sigemptyset(&mut action.sa_mask) } == -1 {
            return Err(std::io::Error::last_os_error().raw_os_error().unwrap_or(0));
        }
        for signal in [libc::SIGINT, libc::SIGHUP, libc::SIGTERM] {
            // SAFETY: `action` remains initialized for the duration of each
            // call, and a null final argument discards the old disposition.
            if unsafe { libc::sigaction(signal, &action, std::ptr::null_mut()) } == -1 {
                return Err(std::io::Error::last_os_error().raw_os_error().unwrap_or(0));
            }
        }
        Ok(())
    });
    result.map_err(|code| std::io::Error::from_raw_os_error(code).into())
}

fn terminated(signal: i32) -> anyhow::Error {
    TerminatedBySignal(signal).into()
}

#[cfg(unix)]
#[derive(Clone, Copy)]
enum HostSupervisorKind {
    Parent,
    TestProcess,
}

#[cfg(unix)]
struct HostSupervisor {
    kind: HostSupervisorKind,
    pid: libc::pid_t,
    #[cfg(target_os = "linux")]
    pidfd: Option<std::os::fd::OwnedFd>,
    #[cfg(target_os = "macos")]
    process_queue: Option<std::os::fd::OwnedFd>,
}

#[cfg(unix)]
impl HostSupervisor {
    fn for_launch(arguments: &LaunchArguments) -> Result<Option<Self>> {
        if arguments.mode != LaunchMode::Serve {
            return Ok(None);
        }
        if let Some(value) = std::env::var_os("RUNYTE_TEST_SUPERVISOR_PID") {
            let pid = value
                .to_string_lossy()
                .parse::<libc::pid_t>()
                .context("RUNYTE_TEST_SUPERVISOR_PID must be a positive process ID")?;
            anyhow::ensure!(pid > 0, "RUNYTE_TEST_SUPERVISOR_PID must be positive");
            return Self::new(HostSupervisorKind::TestProcess, pid).map(Some);
        }
        if arguments.detached_host {
            return Ok(None);
        }
        // SAFETY: `getppid` has no preconditions and only reads process
        // metadata maintained by the kernel.
        Self::new(HostSupervisorKind::Parent, unsafe { libc::getppid() }).map(Some)
    }

    fn new(kind: HostSupervisorKind, pid: libc::pid_t) -> Result<Self> {
        #[cfg(target_os = "linux")]
        let pidfd = open_pidfd(pid)?;
        #[cfg(target_os = "macos")]
        let process_queue = open_process_queue(pid)?;
        Ok(Self {
            kind,
            pid,
            #[cfg(target_os = "linux")]
            pidfd,
            #[cfg(target_os = "macos")]
            process_queue,
        })
    }

    fn exited(&self) -> bool {
        #[cfg(target_os = "linux")]
        if let Some(pidfd) = self.pidfd.as_ref()
            && pidfd_has_exited(pidfd)
        {
            return true;
        }
        #[cfg(target_os = "macos")]
        if let Some(process_queue) = self.process_queue.as_ref()
            && process_queue_has_exited(process_queue)
        {
            return true;
        }
        match self.kind {
            HostSupervisorKind::Parent => {
                // SAFETY: `getppid` has no preconditions.
                (unsafe { libc::getppid() }) != self.pid
            }
            HostSupervisorKind::TestProcess => {
                // SAFETY: signal zero does not deliver a signal; it only asks
                // the kernel whether this positive PID is observable.
                let result = unsafe { libc::kill(self.pid, 0) };
                result == -1 && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH)
                    || process_is_zombie(self.pid)
            }
        }
    }

    fn pid(&self) -> libc::pid_t {
        self.pid
    }
}

#[cfg(target_os = "linux")]
fn open_pidfd(pid: libc::pid_t) -> Result<Option<std::os::fd::OwnedFd>> {
    use std::os::fd::FromRawFd;

    // SAFETY: `pid` is positive and the pidfd syscall takes no pointer
    // arguments. A successful return transfers one fresh descriptor.
    let descriptor = unsafe { libc::syscall(libc::SYS_pidfd_open, pid, 0) };
    if descriptor >= 0 {
        // SAFETY: a non-negative pidfd result is a fresh owned descriptor.
        return Ok(Some(unsafe {
            std::os::fd::OwnedFd::from_raw_fd(descriptor as libc::c_int)
        }));
    }
    let error = std::io::Error::last_os_error();
    match error.raw_os_error() {
        Some(libc::ENOSYS | libc::EINVAL | libc::EPERM) => Ok(None),
        Some(libc::ESRCH) => anyhow::bail!("host supervisor process {pid} already exited"),
        _ => Err(error).context("cannot observe host supervisor process"),
    }
}

#[cfg(target_os = "linux")]
fn pidfd_has_exited(pidfd: &std::os::fd::OwnedFd) -> bool {
    use std::os::fd::AsRawFd;

    let mut descriptor = libc::pollfd {
        fd: pidfd.as_raw_fd(),
        events: libc::POLLIN,
        revents: 0,
    };
    (unsafe { libc::poll(&mut descriptor, 1, 0) }) > 0
        && descriptor.revents & (libc::POLLIN | libc::POLLHUP | libc::POLLERR) != 0
}

#[cfg(target_os = "macos")]
fn open_process_queue(pid: libc::pid_t) -> Result<Option<std::os::fd::OwnedFd>> {
    use std::os::fd::{FromRawFd, OwnedFd};

    // SAFETY: `kqueue` has no preconditions and returns a fresh descriptor.
    let descriptor = unsafe { libc::kqueue() };
    if descriptor == -1 {
        return Err(std::io::Error::last_os_error())
            .context("cannot create host supervisor process queue");
    }
    // SAFETY: a successful `kqueue` call returns a fresh owned descriptor.
    let queue = unsafe { OwnedFd::from_raw_fd(descriptor) };
    let change = libc::kevent {
        ident: pid as libc::uintptr_t,
        filter: libc::EVFILT_PROC,
        flags: libc::EV_ADD | libc::EV_ENABLE | libc::EV_CLEAR,
        fflags: libc::NOTE_EXIT,
        data: 0,
        udata: std::ptr::null_mut(),
    };
    // SAFETY: `queue` is a live kqueue descriptor and `change` points to one
    // fully initialized event registration. No output event list is supplied.
    let status = unsafe {
        libc::kevent(
            descriptor,
            &change,
            1,
            std::ptr::null_mut(),
            0,
            std::ptr::null(),
        )
    };
    if status == 0 {
        return Ok(Some(queue));
    }
    let error = std::io::Error::last_os_error();
    match error.raw_os_error() {
        Some(libc::EPERM) => Ok(None),
        Some(libc::ESRCH | libc::ENOENT) => {
            anyhow::bail!("host supervisor process {pid} already exited")
        }
        _ => Err(error).context("cannot observe host supervisor process"),
    }
}

#[cfg(target_os = "macos")]
fn process_queue_has_exited(process_queue: &std::os::fd::OwnedFd) -> bool {
    use std::os::fd::AsRawFd;

    let mut event = std::mem::MaybeUninit::<libc::kevent>::uninit();
    let timeout = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: the kqueue descriptor is live, the output has capacity for one
    // event, and the zero timeout performs a non-blocking observation.
    (unsafe {
        libc::kevent(
            process_queue.as_raw_fd(),
            std::ptr::null(),
            0,
            event.as_mut_ptr(),
            1,
            &timeout,
        )
    }) > 0
}

#[cfg(target_os = "linux")]
fn process_is_zombie(pid: libc::pid_t) -> bool {
    fs::read_to_string(format!("/proc/{pid}/stat"))
        .ok()
        .and_then(|stat| stat.rsplit_once(") ").map(|(_, suffix)| suffix.to_owned()))
        .and_then(|suffix| suffix.chars().next())
        .is_some_and(|state| matches!(state, 'Z' | 'X'))
}

#[cfg(all(unix, not(target_os = "linux")))]
fn process_is_zombie(_pid: libc::pid_t) -> bool {
    false
}

#[cfg(not(unix))]
struct TerminationSignals;

#[cfg(not(unix))]
impl TerminationSignals {
    fn new() -> Result<Self> {
        Ok(Self)
    }

    async fn recv(&mut self) -> i32 {
        std::future::pending().await
    }
}

async fn run(startup: &mut StartupTrace) -> Result<()> {
    let mut arguments = LaunchArguments::parse()?;
    #[cfg(unix)]
    let supervising_parent = HostSupervisor::for_launch(&arguments)?;
    let show_startup_about = starts_on_about(&arguments);
    startup.mark(StartupPhase::CliParsed);
    if arguments.help {
        print_help();
        return Ok(());
    }
    if arguments.version {
        println!("runyte {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }

    // The documented shell function adds `--cwd-file` to every invocation.
    // Modes without a directory-handoff-capable editor accept it and leave the
    // file untouched so session management remains transparent to the wrapper.
    if matches!(
        arguments.mode,
        LaunchMode::ListSessions
            | LaunchMode::StopAllSessions
            | LaunchMode::ClearAllSessions
            | LaunchMode::RenameSession
    ) || (matches!(
        arguments.mode,
        LaunchMode::StartSession | LaunchMode::RestartSession | LaunchMode::StopSession
    ) && arguments.workspace_selector.is_some())
    {
        // These modes address a host by selector or list every one of them, so
        // they never resolve a project of their own for the option to name.
        anyhow::ensure!(
            arguments.project_root.is_none(),
            "--project-root is not available in this workspace mode"
        );
        #[cfg(unix)]
        {
            return match arguments.mode {
                LaunchMode::ListSessions => {
                    let config = Config::load(arguments.config.as_deref())?.0;
                    list_sessions(&config.workspace.state, arguments.all_namespaces).await
                }
                LaunchMode::StartSession => {
                    let selector = arguments
                        .workspace_selector
                        .as_deref()
                        .expect("selector checked");
                    start_selected_session(
                        selector,
                        arguments.config.as_deref(),
                        arguments.verbosity,
                        arguments.log.as_deref(),
                    )
                    .await
                }
                LaunchMode::StopAllSessions => {
                    let (config, config_path) = Config::load(arguments.config.as_deref())?;
                    stop_all_sessions(
                        &config.workspace.state,
                        config_path.as_deref(),
                        arguments.force,
                        arguments.all_namespaces,
                    )
                    .await
                }
                LaunchMode::ClearAllSessions => {
                    let config = Config::load(arguments.config.as_deref())?.0;
                    let cleared = clear_stopped_sessions(&config.workspace.state).await?;
                    println!(
                        "cleared {cleared} stopped session{}",
                        if cleared == 1 { "" } else { "s" }
                    );
                    Ok(())
                }
                LaunchMode::RenameSession => {
                    let selector = arguments
                        .workspace_selector
                        .as_deref()
                        .expect("parser set selector");
                    let name = arguments
                        .workspace_name
                        .as_deref()
                        .expect("parser set workspace name");
                    rename_selected_session(selector, name, arguments.config.as_deref()).await
                }
                LaunchMode::RestartSession => {
                    let selector = arguments
                        .workspace_selector
                        .as_deref()
                        .expect("selector checked");
                    let (config, config_path) = Config::load(arguments.config.as_deref())?;
                    let endpoint = resolve_lifecycle_endpoint(
                        selector,
                        &config.workspace.state,
                        config_path.as_deref(),
                    )
                    .await?;
                    let startup = HostStartup::new(std::env::current_exe()?, "restarted")
                        .with_config(config_path.as_deref())
                        .with_logging(arguments.verbosity, arguments.log.as_deref());
                    if arguments.force {
                        force_restart_host(&endpoint, startup).await
                    } else {
                        restart_host(&endpoint, startup).await
                    }
                }
                LaunchMode::StopSession => {
                    let selector = arguments
                        .workspace_selector
                        .as_deref()
                        .expect("selector checked");
                    let (config, config_path) = Config::load(arguments.config.as_deref())?;
                    let endpoint = resolve_lifecycle_endpoint(
                        selector,
                        &config.workspace.state,
                        config_path.as_deref(),
                    )
                    .await?;
                    stop_selected_session(&endpoint, arguments.force).await
                }
                _ => unreachable!(),
            };
        }
        #[cfg(not(unix))]
        anyhow::bail!("persistent mode is not yet supported on this platform");
    }

    let (config, config_path) = Config::load(arguments.config.as_deref())?;
    let automatic_persistent = uses_automatic_persistent_mode(&arguments, config.workspace.mode);
    if automatic_persistent {
        #[cfg(unix)]
        {
            arguments.mode = LaunchMode::Persistent;
        }
        #[cfg(not(unix))]
        anyhow::bail!("workspace.mode: persistent is not supported on this platform");
    }
    startup.mark(StartupPhase::ConfigLoaded);
    let launch_directory = std::env::current_dir()?;
    arguments.cwd_file = arguments
        .cwd_file
        .take()
        .map(|path| resolve_cwd_file_path(&launch_directory, path));
    let mut reserved_user_roots = config_path
        .as_deref()
        .map(|path| config::config_root_for(path, &launch_directory))
        .into_iter()
        .collect::<Vec<_>>();
    if let Some(cache_root) = external_open::cache_root() {
        reserved_user_roots.push(cache_root);
    }
    startup.mark(StartupPhase::ProjectResolutionStarted);
    // `-a WORKSPACE` names the workspace outright, so the project this process
    // serves is resolved from the selector rather than from the directory the
    // shell happens to be in. Every lifecycle mode that takes a selector has
    // already returned above, so a selector still present here is an
    // attachment.
    #[cfg(unix)]
    let selected_workspace = match arguments.workspace_selector.take() {
        Some(selector) => Some(
            resolve_attached_workspace(&selector, &launch_directory, &config, &reserved_user_roots)
                .await?,
        ),
        None => None,
    };
    #[cfg(not(unix))]
    let selected_workspace: Option<PathBuf> = {
        anyhow::ensure!(
            arguments.workspace_selector.is_none(),
            "persistent mode is not yet supported on this platform"
        );
        None
    };
    let attaching_elsewhere = selected_workspace.is_some();
    let initializing_current_attachment = arguments.mode == LaunchMode::Persistent
        && arguments.mode_explicit
        && arguments.project_root.is_none()
        && !attaching_elsewhere;
    let initializing = arguments.init.is_some();
    let project_root = match arguments.init.take() {
        Some(requested) => {
            let requested = if requested.is_absolute() {
                requested
            } else {
                launch_directory.join(requested)
            };
            let project_root = project_root::initialize(
                &requested,
                &config.workspace.state,
                &reserved_user_roots,
            )?;
            startup.mark(StartupPhase::ProjectResolvedAutomatically);
            project_root
        }
        None if attaching_elsewhere => {
            let project_root = selected_workspace.expect("selector resolved a workspace");
            startup.mark(StartupPhase::ProjectResolvedAutomatically);
            project_root
        }
        None if initializing_current_attachment => {
            let requested = project_root::discover(&launch_directory, &config.workspace.state)?
                .unwrap_or_else(|| launch_directory.clone());
            let project_root = project_root::initialize(
                &requested,
                &config.workspace.state,
                &reserved_user_roots,
            )?;
            startup.mark(StartupPhase::ProjectResolvedAutomatically);
            project_root
        }
        None => match arguments.project_root.take() {
            // A caller that has already resolved the workspace states it outright.
            // Rediscovering it here would be a second, independent answer to a
            // question that has one right answer per launch, and a detached host
            // has no terminal on which to be asked it again.
            Some(requested) => {
                let project_root = resolve_requested_project_root(&launch_directory, &requested)?;
                startup.mark(StartupPhase::ProjectResolvedAutomatically);
                project_root
            }
            None => match project_root::discover(&launch_directory, &config.workspace.state)? {
                Some(project_root) => {
                    startup.mark(StartupPhase::ProjectResolvedAutomatically);
                    project_root
                }
                None => {
                    let project_root = project_root::prompt(
                        &launch_directory,
                        &config.workspace.state,
                        &reserved_user_roots,
                        runyte::app::user_home_directory().as_deref(),
                        &mut io::stdin().lock(),
                        &mut io::stderr().lock(),
                    )?;
                    startup.mark(StartupPhase::ProjectResolvedAfterPrompt);
                    project_root
                }
            },
        },
    };
    let state_root = project_root::resolve_state_root(&project_root, &config.workspace.state);
    project_root::validate_state_root(&state_root, &reserved_user_roots)?;
    // A selected workspace does not contain the launch directory, so the host
    // it starts is given the workspace's own root. Handing it the directory the
    // shell was in would place a host outside the project it serves.
    let working_directory = if initializing || attaching_elsewhere {
        project_root.clone()
    } else {
        launch_directory.clone()
    };
    if initializing {
        std::env::set_current_dir(&working_directory).with_context(|| {
            format!(
                "cannot enter initialized workspace {}",
                working_directory.display()
            )
        })?;
    }
    #[cfg(unix)]
    let recorded_workspace = if arguments.mode == LaunchMode::Standalone {
        record_recent_workspace(&project_root).ok().flatten()
    } else {
        ensure_recent_workspace(&project_root).ok().flatten()
    };
    let mouse_enabled = config.editor.mouse;
    if matches!(
        arguments.mode,
        LaunchMode::Persistent
            | LaunchMode::Wait
            | LaunchMode::StartSession
            | LaunchMode::RestartSession
            | LaunchMode::StopSession
    ) {
        if arguments.mode != LaunchMode::Wait {
            anyhow::ensure!(
                arguments.targets.is_empty(),
                "this workspace mode does not accept file targets"
            );
        }
        #[cfg(unix)]
        {
            let endpoint = LocalEndpoint::discover(&state_root, &project_root)?;
            let cwd_file = arguments.cwd_file.clone();
            return match arguments.mode {
                LaunchMode::Persistent => {
                    // Persistent mode means "put a TUI on this workspace's
                    // host", which is answerable whether or not one is already
                    // running. Starting the missing host here is what a bare
                    // launch under `workspace.mode: persistent` has always
                    // done.
                    if connect_control(&endpoint).await.is_err() {
                        let startup = HostStartup::new(std::env::current_exe()?, "attached")
                            .with_working_directory(&working_directory)
                            .with_config(config_path.as_deref())
                            .with_logging(arguments.verbosity, arguments.log.as_deref());
                        start_detached_host(&endpoint, startup).await?;
                    } else {
                        report_retained_host_logging(&arguments);
                    }
                    run_workspace_switcher(
                        endpoint,
                        mouse_enabled,
                        cwd_file.as_deref(),
                        &config,
                        config_path.as_deref(),
                    )
                    .await
                }
                LaunchMode::Wait => {
                    run_wait(
                        endpoint,
                        arguments.targets,
                        config_path,
                        mouse_enabled,
                        arguments.verbosity,
                        arguments.log.as_deref(),
                    )
                    .await
                }
                LaunchMode::StartSession => {
                    let startup = HostStartup::new(std::env::current_exe()?, "started")
                        .with_config(config_path.as_deref())
                        .with_logging(arguments.verbosity, arguments.log.as_deref());
                    report_retained_logging_if_running(
                        &endpoint,
                        arguments.verbosity,
                        arguments.log.as_deref(),
                    )
                    .await;
                    ensure_workspace_host(
                        &project_root,
                        &config.workspace.state,
                        config_path.as_deref(),
                        startup,
                    )
                    .await
                    .map(|_| ())
                }
                LaunchMode::RestartSession => {
                    let startup = HostStartup::new(std::env::current_exe()?, "restarted")
                        .with_config(config_path.as_deref())
                        .with_logging(arguments.verbosity, arguments.log.as_deref());
                    if arguments.force {
                        force_restart_host(&endpoint, startup).await
                    } else {
                        restart_host(&endpoint, startup).await
                    }
                }
                LaunchMode::StopSession => {
                    if arguments.force {
                        force_shutdown_host(&endpoint).await
                    } else {
                        shutdown_host(&endpoint).await
                    }
                }
                _ => unreachable!(),
            };
        }
        #[cfg(not(unix))]
        anyhow::bail!("persistent mode is not yet supported on this platform");
    }
    // Diagnostic logging belongs to whichever process owns editor state. A
    // client leaves it uninstalled: its own failures reach stderr once the
    // terminal is restored, and forwarding records would make transport
    // diagnostics depend on the transport being healthy.
    let role = if arguments.mode == LaunchMode::Serve {
        LogRole::Host
    } else {
        LogRole::Standalone
    };
    let logging_failure = initialize_logging(&arguments, role, &state_root, &project_root)?;
    log_info!(
        "process",
        "runyte {} started", env!("CARGO_PKG_VERSION");
        "role" => role,
        "workspace" => workspace_id(&project_root),
        "root" => project_root.display()
    );
    log_debug!(
        "process",
        "diagnostic logging ready";
        "level" => LogLevel::from_verbosity(arguments.verbosity).label(),
        "explicit_destination" => arguments.log.is_some()
    );
    log_trace!(
        "process",
        "resolved workspace locations";
        "state" => state_root.display(),
        "cwd" => launch_directory.display(),
        "config" => config_path.as_deref().map_or_else(
            || "default".to_owned(),
            |path| path.display().to_string(),
        )
    );
    let mut app = App::new_in_project_with_targets_and_trace(
        config,
        arguments.targets,
        project_root.clone(),
        startup,
    )?;
    if let Some(failure) = logging_failure {
        app.push_notification(NotificationDraft::new(
            NotificationSeverity::Warning,
            "Logging",
            "Diagnostic log unavailable",
            format!("{failure} · editing continues without a durable log"),
        ));
    }
    app.set_quit_directory_handoff(arguments.cwd_file.is_some());
    #[cfg(unix)]
    app.note_workspace_number(
        recorded_workspace
            .as_ref()
            .and_then(|recorded| recorded.number),
    );
    if let Some(ref path) = config_path {
        app.note_loaded_config(path);
    }
    // Standalone mode uses the same owner and command/event boundary that a
    // persistent process will host. No transport or daemon is required.
    let mut app = WorkspaceHost::new(app);
    #[cfg(debug_assertions)]
    let mut input_trace = open_input_trace()?;

    if arguments.mode == LaunchMode::Serve {
        #[cfg(unix)]
        {
            let endpoint = LocalEndpoint::discover(&state_root, &project_root)?;
            if let Some(recorded) = recorded_workspace.as_ref() {
                endpoint.store_name_if_absent(&recorded.name)?;
            }
            return run_host_server(
                app,
                endpoint,
                startup,
                config_path.as_deref(),
                supervising_parent,
            )
            .await;
        }
        #[cfg(not(unix))]
        anyhow::bail!("persistent mode is not yet supported on this platform");
    }

    // Register before entering raw mode so no startup interval can leave the
    // terminal modified while signals still have their default disposition.
    let mut termination = TerminationSignals::new()?;
    let mut received_signal = None;
    let _terminal = TerminalGuard::enter(mouse_enabled)?;
    let backend = CrosstermBackend::new(stdout());
    let mut terminal = Terminal::new(backend)?;
    startup.mark(StartupPhase::TerminalEntered);
    let mut key_hints = KeyHintState::default();
    if show_startup_about {
        let invocation = CommandInvocation::editor(
            EditorCommand::ShowAbout,
            CommandExecutionContext::default(),
        )?;
        app.app_mut().execute(invocation)?;
    }
    terminal.draw(|frame| {
        let geometry = ui::frame_geometry(frame.area());
        let snapshot = app.prepare_frame_with_hints(geometry, Some(&key_hints));
        ui::render(frame, app.app(), &snapshot.editor, &key_hints);
    })?;
    startup.mark(StartupPhase::FirstFramePresented);
    if let Err(error) = startup.write_requested() {
        app.report_host_error(format!("failed to write startup timing report: {error}"));
    }

    // Optional services start only after the standalone editor is usable.
    // Their initialization must never hide first-frame latency.
    let mut services = start_host_services(&mut app, startup, config_path.as_deref())?;
    if let Err(error) = startup.write_requested() {
        app.report_host_error(format!("failed to write startup timing report: {error}"));
    }
    // Service discovery can add a useful failure/status message. Present it
    // before waiting for input so a quiet terminal never leaves the initial
    // pre-service frame stale.
    terminal.draw(|frame| {
        let geometry = ui::frame_geometry(frame.area());
        let snapshot = app.prepare_frame_with_hints(geometry, Some(&key_hints));
        ui::render(frame, app.app(), &snapshot.editor, &key_hints);
    })?;
    let mut terminal_events = EventStream::new();
    let mut git_refresh_tick = tokio::time::interval(Duration::from_millis(250));
    git_refresh_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut status_animation_tick = tokio::time::interval(STATUS_ANIMATION_INTERVAL);
    status_animation_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut key_repeat_detector = KeyRepeatDetector::default();
    // Recorded once per service: a channel that closes stays closed, and the
    // editor keeps working without it, so nothing else reports the loss.
    let mut ended_services: std::collections::HashSet<&'static str> =
        std::collections::HashSet::new();
    loop {
        key_hints.expire_at(Instant::now());
        if app.should_quit {
            break;
        }
        let hint_timeout = key_hints.time_until_expiry(Instant::now());
        tokio::select! {
            input = terminal_events.next() => {
                match input.transpose()? {
                    // Fall through to the draw at the bottom of the loop
                    // rather than taking the lifecycle `continue` below.
                    Some(event) if is_redraw_only_event(&event) => {
                        key_repeat_detector.observe(None, None, Instant::now());
                    }
                    Some(event) => {
                        let key_kind = terminal_key_kind(&event);
                        let Some(input) = convert_event(event)? else {
                            key_repeat_detector.observe(key_kind, None, Instant::now());
                            continue;
                        };
                        let repeated = key_repeat_detector.observe(
                            key_kind,
                            Some(&input),
                            Instant::now(),
                        );
                        if let Some(message) = rejected_text_input(&input) {
                            app.report_host_error(message);
                            terminal.draw(|frame| {
                                let geometry = ui::frame_geometry(frame.area());
                                let snapshot = app.prepare_frame_with_hints(
                                    geometry,
                                    Some(&key_hints),
                                );
                                ui::render(frame, app.app(), &snapshot.editor, &key_hints);
                            })?;
                            continue;
                        }
                        #[cfg(debug_assertions)]
                        trace_input(
                            input_trace.as_mut(),
                            "before",
                            app.app(),
                            &input,
                            repeated,
                            None,
                        )?;
                        if is_passive_pointer(&input) {
                            // Passive motion from Crossterm's any-motion mode
                            // is not editor input. Preserve hints/status and
                            // avoid a full semantic/render cycle.
                            continue;
                        }
                        let hint_result = match &input {
                            InputEvent::Pointer(event) => {
                                key_hints.clear();
                                if let Some(frame) = app.current_frame_id() {
                                    match app.execute(HostCommand::Pointer {
                                        event: *event,
                                        frame,
                                        repetitions: 1,
                                    }) {
                                        Ok(
                                            HostInputOutcome::Applied
                                            | HostInputOutcome::AppliedWithoutVisualChange
                                            | HostInputOutcome::IgnoredStaleFrame,
                                        ) => {}
                                        Err(error) => {
                                            app.report_host_error(error.to_string());
                                        }
                                    }
                                }
                                HintEventResult::Consumed
                            }
                            InputEvent::Key(_) | InputEvent::Text(_) => {
                                observe_key_or_text_hint(app.app(), &mut key_hints, &input)
                            }
                        };
                        if hint_result == HintEventResult::Forward {
                            let dispatches = motion_repeat_dispatches(&app, &input, repeated);
                            for _ in 0..dispatches {
                                if let Err(error) = app.execute(HostCommand::Input(input.clone())) {
                                    app.report_host_error(error.to_string());
                                    break;
                                }
                            }
                        }
                        #[cfg(debug_assertions)]
                        trace_input(
                            input_trace.as_mut(),
                            "after",
                            app.app(),
                            &input,
                            repeated,
                            Some(hint_result),
                        )?;
                    }
                    None => break,
                }
            }
            event = services.lsp_events.recv() => {
                if let Some(event) = event {
                    app.apply_event(HostEvent::Lsp(event));
                } else {
                    note_ended_service(&mut ended_services, "language servers");
                }
            }
            event = services.syntax_events.recv() => {
                if let Some(event) = event {
                    app.apply_event(HostEvent::Syntax(event));
                } else {
                    note_ended_service(&mut ended_services, "syntax");
                }
            }
            event = services.file_picker_events.recv() => {
                if let Some(event) = event {
                    app.apply_event(HostEvent::FilePicker(event));
                }
            }
            event = services.file_monitor_events.recv() => {
                if let Some(event) = event {
                    app.apply_event(HostEvent::FileObservation(event));
                } else {
                    note_ended_service(&mut ended_services, "file monitor");
                }
            }
            output = services.terminal_events.recv() => {
                if let Some(output) = output {
                    app.apply_event(HostEvent::Terminal(output));
                    terminal::drain(&mut services.terminal_events, |output| {
                        app.apply_event(HostEvent::Terminal(output));
                    });
                }
            }
            event = receive_workspace_event(&mut services.workspace_events) => {
                if let Some(event) = event {
                    app.apply_event(event);
                }
            }
            event = async {
                match services.git_events.as_mut() {
                    Some(events) => events.recv().await,
                    None => std::future::pending().await,
                }
            } => {
                if let Some(event) = event {
                    app.apply_event(HostEvent::Git(event));
                }
            }
            _ = git_refresh_tick.tick() => {
                report_logging_failure(app.app_mut());
                services.file_monitor.sync(app.file_monitor_requests());
                let changed = app.refresh_git_if_due(Instant::now());
                let activity_changed = app.refresh_session_activity();
                if !changed && !activity_changed {
                    continue;
                }
            }
            _ = status_animation_tick.tick(), if app.has_long_running_action() => {}
            _ = tokio::task::yield_now(), if app.macro_replay_pending() => {
                if let Err(error) = app.advance_macro_replay() {
                    app.report_host_error(error.to_string());
                }
            }
            _ = tokio::time::sleep(hint_timeout.unwrap_or_default()), if hint_timeout.is_some() => {
                key_hints.expire_at(Instant::now());
            }
            signal = termination.recv() => {
                received_signal = Some(signal);
                break;
            }
        }
        terminal.draw(|frame| {
            let geometry = ui::frame_geometry(frame.area());
            let snapshot = app.prepare_frame_with_hints(geometry, Some(&key_hints));
            ui::render(frame, app.app(), &snapshot.editor, &key_hints);
        })?;
    }
    let quit_directory = app.quit_directory().map(Path::to_path_buf);
    services.language_servers.send(LspCommand::Shutdown);
    let cwd_file = arguments.cwd_file;
    if let (Some(cwd_file), Some(directory)) = (cwd_file.as_deref(), quit_directory) {
        write_cwd_file(cwd_file, &directory)?;
    }
    if let Some(signal) = received_signal {
        return Err(terminated(signal));
    }
    Ok(())
}

fn starts_on_about(arguments: &LaunchArguments) -> bool {
    arguments.mode == LaunchMode::Standalone && arguments.targets.is_empty()
}

fn uses_automatic_persistent_mode(
    arguments: &LaunchArguments,
    workspace_mode: WorkspaceMode,
) -> bool {
    // The persistent default is deliberately a bare-launch convenience. A
    // target may carry a caller-relative path or an initial caret position,
    // and the attach protocol does not represent all of those launch
    // semantics. Keep target-bearing invocations on the ordinary standalone
    // path unless a future protocol can preserve the complete target.
    !arguments.mode_explicit
        && arguments.targets.is_empty()
        && workspace_mode == WorkspaceMode::Persistent
}

/// Resolves the workspace a persistent attachment named on the command line.
///
/// The selector is matched against the workspace catalog first, so an ID, an
/// unambiguous ID prefix, a persistent name, or a known root attaches without
/// consulting the filesystem. A directory the catalog does not know names that
/// exact directory. It is initialized as a workspace when necessary, so
/// attachment never needs the interactive project-root prompt and does not
/// silently collapse a named nested directory into an ancestor workspace.
#[cfg(unix)]
async fn resolve_attached_workspace(
    selector: &Path,
    working_directory: &Path,
    config: &Config,
    reserved_user_roots: &[PathBuf],
) -> Result<PathBuf> {
    let requested = resolve_known_workspace_from_directory(
        selector,
        working_directory,
        &config.workspace.state,
    )
    .await?
    .unwrap_or_else(|| workspace_selector_path(selector, working_directory));
    initialize_attached_directory(
        &requested,
        selector,
        &config.workspace.state,
        reserved_user_roots,
    )
}

/// Resolves and initializes a directory selected for attachment.
///
/// `display_selector` remains the spelling in an unknown-selector error even
/// when a relative path has already been joined to the editor directory.
fn initialize_attached_directory(
    requested: &Path,
    display_selector: &Path,
    state: &Path,
    reserved_user_roots: &[PathBuf],
) -> Result<PathBuf> {
    let unknown = || {
        anyhow::anyhow!(
            "no session matches {}; use --session-list to see available sessions",
            display_selector.display()
        )
    };
    let directory = requested.canonicalize().map_err(|_| unknown())?;
    anyhow::ensure!(directory.is_dir(), unknown());
    project_root::initialize(&directory, state, reserved_user_roots)
}

fn workspace_selector_path(selector: &Path, working_directory: &Path) -> PathBuf {
    if selector.is_absolute() {
        selector.to_path_buf()
    } else {
        working_directory.join(selector)
    }
}

/// Accepts a caller-resolved workspace root, or explains why it cannot be one.
///
/// The check mirrors the one [`start_detached_host`] applies to the working
/// directory it spawns a host in: a workspace owns every directory below it, so
/// a root that does not contain the launch directory would give this process a
/// different project from the one it is running in. Failing here keeps that
/// mismatch from reaching workspace identity, which is derived from the root.
fn resolve_requested_project_root(launch_directory: &Path, requested: &Path) -> Result<PathBuf> {
    let project_root = requested
        .canonicalize()
        .with_context(|| format!("cannot resolve project root {}", requested.display()))?;
    anyhow::ensure!(
        project_root.is_dir(),
        "project root {} is not a directory",
        project_root.display()
    );
    anyhow::ensure!(
        launch_directory.starts_with(&project_root),
        "launch directory {} is outside project root {}",
        launch_directory.display(),
        project_root.display()
    );
    Ok(project_root)
}

/// Gives the shell handoff file a process-independent identity.
///
/// Persistent attachments may move between project roots while the client
/// process keeps running. Resolving a relative `--cwd-file` before attachment
/// begins keeps every workspace writing the file the invoking shell awaits.
fn resolve_cwd_file_path(invocation_directory: &Path, path: PathBuf) -> PathBuf {
    if path.is_absolute() {
        path
    } else {
        invocation_directory.join(path)
    }
}

#[cfg(unix)]
struct AttachedClient {
    id: u64,
    geometry: runyte::app::FrameGeometry,
    responses: runyte::workspace::transport::ResponseSender,
    wait_tokens: Vec<WaitToken>,
    last_frame: Option<runyte::protocol::HostFrame>,
}

#[cfg(unix)]
async fn run_host_server(
    mut host: WorkspaceHost,
    endpoint: LocalEndpoint,
    startup: &mut StartupTrace,
    config_path: Option<&Path>,
    supervising_parent: Option<HostSupervisor>,
) -> Result<()> {
    let mut termination = TerminationSignals::new()?;
    host.enable_persistent_session();
    let mut server = LocalServer::bind(&endpoint).await?;
    log_info!(
        "host",
        "persistent session published";
        "workspace" => endpoint.id(),
        "socket" => endpoint.socket().display()
    );
    let mut services = start_host_services(&mut host, startup, config_path)?;
    let mut last_detached = Instant::now();
    let mut idle_tick = tokio::time::interval(Duration::from_secs(1));
    idle_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut active: Option<AttachedClient> = None;
    let mut controls: std::collections::HashMap<u64, runyte::workspace::transport::ResponseSender> =
        std::collections::HashMap::new();
    let mut refresh_tick = tokio::time::interval(Duration::from_millis(250));
    refresh_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut status_animation_tick = tokio::time::interval(STATUS_ANIMATION_INTERVAL);
    status_animation_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut terminal_frame_tick = tokio::time::interval(TERMINAL_FRAME_INTERVAL);
    terminal_frame_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut terminal_frame_pending = false;
    let mut key_hints = KeyHintState::default();
    let mut shutting_down = false;
    let mut received_signal = None;
    // A background service whose channel closes is gone for the rest of this
    // host's life, and a detached host has no other way to say so.
    let mut ended_services: std::collections::HashSet<&'static str> =
        std::collections::HashSet::new();
    while !shutting_down {
        key_hints.expire_at(Instant::now());
        let mut changed = false;
        let hint_timeout = key_hints.time_until_expiry(Instant::now());
        tokio::select! {
            event = server.recv() => {
                let Some(event) = event else {
                    log_error!("host", "the connection listener stopped; this session cannot be reached again");
                    anyhow::bail!("workspace host listener stopped unexpectedly");
                };
                match event {
                    ServerEvent::Connected { id, geometry, interactive, directory_handoff, responses } => {
                        if interactive && active.is_some() {
                            log_warn!(
                                "client",
                                "refused a second interactive attachment";
                                "connection" => id
                            );
                            let _ = responses.try_send(HostResponse::Refused {
                                message: "another interactive TUI is already attached".to_owned(),
                            });
                        } else if interactive {
                            // `:quit-here` is only meaningful while a client
                            // that can reach a shell is attached, and each
                            // client is launched separately, so the capability
                            // follows the attachment rather than the host.
                            host.set_quit_directory_handoff(directory_handoff);
                            let client = AttachedClient {
                                id,
                                geometry,
                                responses,
                                wait_tokens: Vec::new(),
                                last_frame: None,
                            };
                            if client.responses.try_send(HostResponse::Welcome {
                                protocol: runyte::workspace::transport::PROTOCOL_VERSION,
                                pid: std::process::id(),
                                features: vec![
                                    FeatureGroup::Snapshots,
                                    FeatureGroup::Input,
                                    FeatureGroup::Buffers,
                                    FeatureGroup::Wait,
                                ],
                                host_version: env!("CARGO_PKG_VERSION").to_owned(),
                            }).is_ok() {
                                active = Some(client);
                                last_detached = Instant::now();
                                host.app_mut().note_frontend_attached();
                                log_info!(
                                    "client",
                                    "interactive client attached";
                                    "connection" => id
                                );
                                publish_attached_frame(&mut host, &mut active, &key_hints);
                                terminal_frame_pending = false;
                            }
                        } else if responses.try_send(HostResponse::Welcome {
                            protocol: runyte::workspace::transport::PROTOCOL_VERSION,
                            pid: std::process::id(),
                            features: vec![
                                FeatureGroup::Control,
                                FeatureGroup::Buffers,
                                FeatureGroup::Wait,
                            ],
                            host_version: env!("CARGO_PKG_VERSION").to_owned(),
                        }).is_ok() {
                            log_debug!("client", "control client attached"; "connection" => id);
                            controls.insert(id, responses);
                        }
                    }
                    ServerEvent::Request { id, request } => {
                        let interactive = active.as_ref().is_some_and(|client| client.id == id);
                        let control = controls.contains_key(&id);
                        if !interactive && !control {
                            continue;
                        }
                        if control {
                            if matches!(request, ClientRequest::Shutdown) {
                                let protected = host.protected_state();
                                if !protected.is_empty() {
                                    send_control_response(
                                        &mut controls,
                                        id,
                                        HostResponse::Refused {
                                            message: protected.refusal(),
                                        },
                                    );
                                } else {
                                    if let Some(responses) = controls.get(&id) {
                                        let _ = responses.try_send(HostResponse::ShuttingDown);
                                    }
                                    shutting_down = true;
                                }
                            } else if matches!(request, ClientRequest::ForceShutdown) {
                                log_warn!(
                                    "host",
                                    "forced termination discarded protected state";
                                    "connection" => id
                                );
                                if let Some(responses) = controls.get(&id) {
                                    let _ = responses.try_send(HostResponse::ShuttingDown);
                                }
                                shutting_down = true;
                            } else if let ClientRequest::RenameHost { name } = &request {
                                let response = endpoint.rename(name).map_or_else(
                                    |error| HostResponse::Error {
                                        message: error.to_string(),
                                    },
                                    |()| HostResponse::HostRenamed { name: name.clone() },
                                );
                                send_control_response(&mut controls, id, response);
                            } else if let Some(reply) = handle_workspace_request(
                                &mut host,
                                request,
                                active.is_some(),
                                false,
                            ) {
                                if let HostResponse::WaitCreated { token, .. } = &reply.response
                                    && let Some(client) = active.as_mut()
                                    && !client.wait_tokens.contains(token)
                                {
                                    client.wait_tokens.push(*token);
                                }
                                send_control_response(&mut controls, id, reply.response);
                                changed |= reply.publish_frame;
                            }
                        } else if let ClientRequest::AttachWait { token } = request {
                            let response = match host.wait_status(token.into()) {
                                Some(status) => {
                                    if let Some(client) = active.as_mut()
                                        && !client.wait_tokens.contains(&token)
                                    {
                                        client.wait_tokens.push(token);
                                    }
                                    HostResponse::WaitState {
                                        token,
                                        status: status.into(),
                                        interactive_attached: true,
                                    }
                                }
                                None => HostResponse::Error {
                                    message: format!("unknown wait token {token}"),
                                },
                            };
                            send_active_response(&mut active, response);
                        } else if is_workspace_request(&request) {
                            if let Some(reply) = handle_workspace_request(
                                &mut host,
                                request,
                                true,
                                true,
                            ) {
                                if let HostResponse::WaitCreated { token, .. } = &reply.response
                                    && let Some(client) = active.as_mut()
                                    && !client.wait_tokens.contains(token)
                                {
                                    client.wait_tokens.push(*token);
                                }
                                send_active_response(&mut active, reply.response);
                                changed |= reply.publish_frame;
                            }
                        } else {
                            match request {
                            ClientRequest::Input { event, repeated } => {
                                dispatch_host_key_or_text(
                                    &mut host,
                                    &mut key_hints,
                                    event.into(),
                                    repeated,
                                );
                                host.reconcile_wait_requests();
                                changed = true;
                            }
                            ClientRequest::Pointer {
                                event,
                                frame,
                                repetitions,
                            } => {
                                key_hints.clear();
                                match host.execute(HostCommand::Pointer {
                                    event: event.into(),
                                    frame: frame.into(),
                                    repetitions,
                                }) {
                                    Ok(HostInputOutcome::Applied) => changed = true,
                                    Ok(
                                        HostInputOutcome::AppliedWithoutVisualChange
                                        | HostInputOutcome::IgnoredStaleFrame,
                                    ) => {}
                                    Err(error) => host.report_host_error(error.to_string()),
                                }
                            }
                            ClientRequest::Resize { geometry } => {
                                if let Some(client) = active.as_mut() {
                                    client.geometry = geometry.into();
                                }
                                changed = true;
                            }
                            ClientRequest::Resynchronize => {
                                if let Some(client) = active.as_mut() {
                                    client.last_frame = None;
                                }
                                changed = true;
                            }
                            ClientRequest::Detach => {
                                log_info!(
                                    "client",
                                    "interactive client detached";
                                    "connection" => id
                                );
                                key_hints.clear();
                                // An explicit detach request is not `:quit-here`,
                                // so it never carries a directory handoff.
                                detach_client(&mut active, None);
                                last_detached = Instant::now();
                            }
                            ClientRequest::Shutdown => {
                                let protected = host.protected_state();
                                if protected.is_empty() {
                                    if let Some(client) = active.as_ref() {
                                        let _ = client
                                            .responses
                                            .try_send(HostResponse::ShuttingDown);
                                    }
                                    shutting_down = true;
                                } else if let Some(client) = active.as_ref() {
                                    let _ = client.responses.try_send(HostResponse::Refused {
                                        message: protected.refusal(),
                                    });
                                }
                            }
                            ClientRequest::ForceShutdown => {
                                log_warn!(
                                    "host",
                                    "forced termination discarded protected state";
                                    "connection" => id
                                );
                                if let Some(client) = active.as_ref() {
                                    let _ = client.responses.try_send(HostResponse::ShuttingDown);
                                }
                                shutting_down = true;
                            }
                            ClientRequest::Notify { message } => {
                                // Something the client discovered on its own, such
                                // as a destination workspace it could not reach.
                                // The editor on screen is the only surface it has.
                                host.report_host_error(message);
                                changed = true;
                            }
                            ClientRequest::Hello { .. } => {}
                            ClientRequest::Invoke { .. }
                            | ClientRequest::Health
                            | ClientRequest::SessionPreview
                            | ClientRequest::ListBuffers
                            | ClientRequest::ReadBuffer { .. }
                            | ClientRequest::OpenBuffers { .. }
                            | ClientRequest::ApplyTransaction { .. }
                            | ClientRequest::SaveBuffer { .. }
                            | ClientRequest::CloseBuffer { .. }
                            | ClientRequest::CreateWait { .. }
                            | ClientRequest::AttachWait { .. }
                            | ClientRequest::WaitStatus { .. }
                            | ClientRequest::CompleteWaitBuffer { .. }
                            | ClientRequest::CancelWait { .. }
                            | ClientRequest::RenameHost { .. } => {}
                            }
                        }
                    }
                    ServerEvent::ProtocolError { id, message } => {
                        // The one place that still knows which connection sent
                        // a malformed or truncated frame, and what preceded it.
                        log_warn!(
                            "transport",
                            "rejected a client frame: {message}";
                            "connection" => id,
                            "interactive" => active.as_ref().is_some_and(|client| client.id == id)
                        );
                        let response = HostResponse::Error { message };
                        if active.as_ref().is_some_and(|client| client.id == id) {
                            send_active_response(&mut active, response);
                        } else {
                            send_control_response(&mut controls, id, response);
                        }
                    }
                    ServerEvent::TransportFailure { id, message } => {
                        // A framing error ends the connection, so no response
                        // is sent and no further request will arrive. This is
                        // the only place a malformed or truncated frame is
                        // named; the `Disconnected` that follows only says the
                        // connection went away.
                        log_warn!(
                            "transport",
                            "client connection failed: {message}";
                            "connection" => id,
                            "interactive" => active.as_ref().is_some_and(|client| client.id == id)
                        );
                    }
                    ServerEvent::Disconnected { id } => {
                        let control = controls.remove(&id).is_some();
                        if active.as_ref().is_some_and(|client| client.id == id) {
                            key_hints.clear();
                            active = None;
                            last_detached = Instant::now();
                            log_info!(
                                "client",
                                "interactive client disconnected";
                                "connection" => id
                            );
                        } else if control {
                            log_debug!("client", "control client disconnected"; "connection" => id);
                        } else {
                            // Either a connection refused at handshake, or an
                            // interactive one whose closure was already
                            // recorded where the failing write observed it.
                            log_debug!("client", "connection closed"; "connection" => id);
                        }
                    }
                }
            }
            event = services.lsp_events.recv() => {
                if let Some(event) = event {
                    host.apply_event(HostEvent::Lsp(event));
                    changed = true;
                } else {
                    note_ended_service(&mut ended_services, "language servers");
                }
            }
            event = services.syntax_events.recv() => {
                if let Some(event) = event {
                    host.apply_event(HostEvent::Syntax(event));
                    changed = true;
                } else {
                    note_ended_service(&mut ended_services, "syntax");
                }
            }
            event = services.file_picker_events.recv() => {
                if let Some(event) = event {
                    host.apply_event(HostEvent::FilePicker(event));
                    changed = true;
                }
            }
            event = services.file_monitor_events.recv() => {
                if let Some(event) = event {
                    host.apply_event(HostEvent::FileObservation(event));
                    changed = true;
                } else {
                    note_ended_service(&mut ended_services, "file monitor");
                }
            }
            output = services.terminal_events.recv() => {
                if let Some(output) = output {
                    let observed = active.is_some();
                    host.apply_terminal_output(output, observed);
                    terminal::drain(&mut services.terminal_events, |output| {
                        host.apply_terminal_output(output, observed);
                    });
                    terminal_frame_pending = true;
                }
            }
            event = receive_workspace_event(&mut services.workspace_events) => {
                if let Some(event) = event {
                    host.apply_event(event);
                    changed = true;
                }
            }
            event = async {
                match services.git_events.as_mut() {
                    Some(events) => events.recv().await,
                    None => std::future::pending().await,
                }
            } => {
                if let Some(event) = event {
                    host.apply_event(HostEvent::Git(event));
                    changed = true;
                }
            }
            _ = refresh_tick.tick() => {
                report_logging_failure(host.app_mut());
                services.file_monitor.sync(host.file_monitor_requests());
                changed = host.refresh_git_if_due(Instant::now());
                changed |= host.refresh_session_activity();
            }
            _ = idle_tick.tick() => {
                if supervising_parent
                    .as_ref()
                    .is_some_and(HostSupervisor::exited)
                {
                    log_info!(
                        "host",
                        "foreground supervisor exited; retiring persistent session";
                        "parent" => supervising_parent.as_ref().expect("checked as some").pid()
                    );
                    shutting_down = true;
                    continue;
                }
                // Read per tick rather than once at startup: the settings view
                // applies this immediately, and a host that had to be restarted
                // to honor its own retirement interval would be answering with
                // the very thing the interval decides.
                let idle_retirement = Duration::from_secs(
                    (host.config.workspace.idle_retirement_minutes as u64).saturating_mul(60),
                );
                if !idle_retirement.is_zero()
                    && active.is_none()
                    && host.may_retire_idle()
                    && Instant::now().saturating_duration_since(last_detached) >= idle_retirement
                {
                    log_info!(
                        "host",
                        "retiring after an idle interval";
                        "minutes" => host.config.workspace.idle_retirement_minutes
                    );
                    shutting_down = true;
                }
            }
            _ = status_animation_tick.tick(), if host.has_long_running_action() => {
                changed = true;
            }
            _ = tokio::task::yield_now(), if host.macro_replay_pending() => {
                if let Err(error) = host.advance_macro_replay() {
                    host.report_host_error(error.to_string());
                }
                changed = true;
            }
            _ = terminal_frame_tick.tick(), if terminal_frame_pending && active.is_some() => {
                changed = true;
            }
            _ = async {
                match hint_timeout {
                    Some(timeout) => tokio::time::sleep(timeout).await,
                    None => std::future::pending().await,
                }
            } => {
                key_hints.expire_at(Instant::now());
                changed = true;
            }
            signal = termination.recv() => {
                log_warn!("host", "terminated by a signal"; "signal" => signal);
                received_signal = Some(signal);
                shutting_down = true;
            }
        }
        // Lifecycle requests may be completed by a background service rather
        // than by the input event that started them. In particular, worktree
        // creation asks to switch only after the asynchronous Git mutation is
        // definitively successful. Drain before accepting another input so no
        // key can land in a workspace the client has already asked to leave.
        if let Some(root) = host.take_workspace_switch() {
            // A background operation may finish after its initiating TUI has
            // disconnected. Consume that stale request here so the next,
            // unrelated attachment is not switched out from under itself.
            if active.is_some() {
                key_hints.clear();
                switch_attached_workspace(&mut host, &mut active, root);
                last_detached = Instant::now();
                changed = false;
            }
        } else if let Some(request) = host.take_persistent_exit_request()
            && active.is_some()
        {
            key_hints.clear();
            match request {
                PersistentExitRequest::Detach => {
                    finish_attached_detach(&mut host, &mut active);
                    last_detached = Instant::now();
                    changed = false;
                }
                PersistentExitRequest::Quit { force } => {
                    if finish_attached_quit(&mut host, &mut active, force) {
                        shutting_down = true;
                        changed = false;
                    } else {
                        changed = true;
                    }
                }
            }
        }
        if changed {
            publish_attached_frame(&mut host, &mut active, &key_hints);
            terminal_frame_pending = false;
        }
    }
    log_info!("host", "persistent session shutting down"; "workspace" => endpoint.id());
    host.cancel_all_waits("workspace host shut down");
    services.language_servers.send(LspCommand::Shutdown);
    // Unpublish before flushing rather than after: the listener is still
    // accepting while the connections that are already established finish,
    // and a client that discovered this endpoint in that window would attach
    // to a host with no loop left to answer it.
    let unpublished = endpoint.cleanup();
    if let Err(error) = &unpublished {
        log_error!("host", "could not retire the published endpoint: {error}");
    }
    flush_connections(&mut server, active, controls).await;
    log_info!("host", "connections flushed and endpoint retired");
    diagnostic_log::flush(diagnostic_log::FLUSH_BUDGET);
    unpublished?;
    if let Some(signal) = received_signal {
        return Err(terminated(signal));
    }
    Ok(())
}

/// Lets every connection finish the message it is writing before the process
/// that owns it exits.
///
/// A connection task writes one framed message at a time, and the runtime
/// stops as soon as this function's caller returns. Leaving a write in flight
/// truncates it, so the client reads a message that ends inside itself and
/// reports a transport error for what is an ordinary shutdown. Dropping the
/// response senders closes each channel, which lets the task deliver what is
/// already queued — `ShuttingDown` included — and then close its socket at a
/// frame boundary. Waiting for the resulting `Disconnected` events keeps the
/// runtime alive until that has happened. The budget bounds a peer that has
/// stopped reading: it loses its last message, exactly as it did before.
#[cfg(unix)]
async fn flush_connections(
    server: &mut LocalServer,
    active: Option<AttachedClient>,
    controls: std::collections::HashMap<u64, runyte::workspace::transport::ResponseSender>,
) {
    let mut pending: std::collections::HashSet<u64> = controls.keys().copied().collect();
    pending.extend(active.as_ref().map(|client| client.id));
    drop(active);
    drop(controls);
    let deadline = tokio::time::sleep(SHUTDOWN_FLUSH_BUDGET);
    tokio::pin!(deadline);
    while !pending.is_empty() {
        tokio::select! {
            () = &mut deadline => break,
            event = server.recv() => match event {
                Some(ServerEvent::Disconnected { id }) => {
                    pending.remove(&id);
                }
                Some(_) => {}
                None => break,
            },
        }
    }
}

#[cfg(unix)]
fn publish_attached_frame(
    host: &mut WorkspaceHost,
    active: &mut Option<AttachedClient>,
    key_hints: &KeyHintState,
) {
    let Some(client) = active.as_mut() else {
        return;
    };
    host.mark_visible_terminals_viewed();
    let frame: runyte::protocol::HostFrame = host
        .prepare_frame_with_hints(client.geometry, Some(key_hints))
        .into();
    let response = if client.responses.visual_pending() {
        // Replacing an unseen delta with another delta would make the latter's
        // base impossible for the client to have. A complete replacement is
        // still one bounded slot and lets the client converge without a
        // resynchronization loop under continuous output.
        HostResponse::Frame {
            frame: Box::new(frame.clone()),
        }
    } else {
        client
            .last_frame
            .as_ref()
            .and_then(|base| runyte::protocol::TerminalDamageFrame::between(base, &frame))
            .map_or_else(
                || HostResponse::Frame {
                    frame: Box::new(frame.clone()),
                },
                |damage| HostResponse::TerminalDamage {
                    damage: Box::new(damage),
                },
            )
    };
    // A frame is a whole snapshot, so a client that cannot keep up loses
    // nothing by missing one: the next publish supersedes it. Only a closed
    // connection means the client is actually gone. Detaching on a merely
    // full channel used to end the session mid-keystroke, which reached the
    // person as an unexplained clean exit.
    match client.responses.try_send(response) {
        Ok(()) => client.last_frame = Some(frame),
        Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => {
            // The write is where the closure is observed, so this is the
            // boundary that records it. The `Disconnected` event that follows
            // finds no attachment left and stays quiet rather than reporting
            // the same departure twice.
            log_info!(
                "client",
                "interactive client disconnected";
                "connection" => client.id,
                "observed" => "frame publication"
            );
            *active = None;
        }
        Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => {}
    }
}

#[cfg(unix)]
fn dispatch_host_key_or_text(
    host: &mut WorkspaceHost,
    key_hints: &mut KeyHintState,
    input: InputEvent,
    repeated: bool,
) {
    let hint_result = observe_key_or_text_hint(host.app(), key_hints, &input);
    if hint_result != HintEventResult::Forward {
        return;
    }
    let dispatches = motion_repeat_dispatches(host.app(), &input, repeated);
    for _ in 0..dispatches {
        if let Err(error) = host.execute(HostCommand::Input(input.clone())) {
            host.report_host_error(error.to_string());
            break;
        }
    }
}

fn observe_key_or_text_hint(
    app: &App,
    key_hints: &mut KeyHintState,
    input: &InputEvent,
) -> HintEventResult {
    if app.macro_replay_pending() {
        key_hints.clear();
        return HintEventResult::Forward;
    }
    match input {
        InputEvent::Key(key) if !app.has_input_overlay() => {
            observe_editor_key_hint(app, key_hints, *key)
        }
        InputEvent::Key(_) | InputEvent::Text(_) => {
            key_hints.clear();
            HintEventResult::Forward
        }
        InputEvent::Pointer(_) => {
            key_hints.clear();
            HintEventResult::Consumed
        }
    }
}

fn observe_editor_key_hint(
    app: &App,
    key_hints: &mut KeyHintState,
    key: KeyStroke,
) -> HintEventResult {
    let Some(mode) = app.key_hint_mode_for_key(key) else {
        key_hints.clear();
        return HintEventResult::Forward;
    };
    key_hints.observe_in(key, mode, app.key_binding_scope(), app.keymap())
}

/// Opens the opt-in development trace used to diagnose native key dispatch.
///
/// `InputEvent` deliberately redacts pasted/composed text in its `Debug`
/// representation. The remaining state is limited to key metadata and the
/// terminal pane transition needed for this diagnosis.
#[cfg(debug_assertions)]
fn open_input_trace() -> Result<Option<fs::File>> {
    let Some(path) = std::env::var_os("RUNYTE_INPUT_TRACE") else {
        return Ok(None);
    };
    fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(&path)
        .with_context(|| {
            format!(
                "failed to open RUNYTE_INPUT_TRACE path {}",
                Path::new(&path).display()
            )
        })
        .map(Some)
}

#[cfg(debug_assertions)]
fn trace_input(
    trace: Option<&mut fs::File>,
    phase: &str,
    app: &App,
    input: &InputEvent,
    repeated: bool,
    hint: Option<HintEventResult>,
) -> Result<()> {
    let Some(trace) = trace else {
        return Ok(());
    };
    let terminal = app.active_terminal();
    let reviewing = terminal
        .and_then(|id| app.terminals.get(id))
        .is_some_and(|session| session.reviewing());
    writeln!(
        trace,
        "{phase} input={input:?} repeated={repeated} hint={hint:?} mode={:?} pane={} \
         terminal={terminal:?} reviewing={reviewing} pending={} fast_pane_keys={}",
        app.mode,
        app.active_pane,
        app.pending_sequence(),
        app.config.editor.fast_pane_keys,
    )
    .context("failed to write RUNYTE_INPUT_TRACE")?;
    trace.flush().context("failed to flush RUNYTE_INPUT_TRACE")
}

#[cfg(unix)]
fn is_workspace_request(request: &ClientRequest) -> bool {
    matches!(
        request,
        ClientRequest::Invoke { .. }
            | ClientRequest::Health
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
    )
}

#[cfg(unix)]
struct WorkspaceReply {
    response: HostResponse,
    publish_frame: bool,
}

#[cfg(unix)]
fn workspace_response_publishes_frame(response: &HostResponse) -> bool {
    matches!(
        response,
        HostResponse::CommandResult { .. }
            | HostResponse::Opened { .. }
            | HostResponse::TransactionApplied { .. }
            | HostResponse::Saved { .. }
            | HostResponse::Closed { .. }
            | HostResponse::WaitCreated { .. }
    )
}

#[cfg(unix)]
fn handle_workspace_request(
    host: &mut WorkspaceHost,
    request: ClientRequest,
    interactive_attached: bool,
    allow_invoke: bool,
) -> Option<WorkspaceReply> {
    use runyte::{
        command::parse_named_command,
        text::{Change, Transaction},
        workspace::BufferRequestError,
    };

    let result = match request {
        ClientRequest::Health => Ok(HostResponse::Health {
            protocol: runyte::workspace::transport::PROTOCOL_VERSION,
            pid: std::process::id(),
            interactive_attached,
            unsaved_buffers: host.protected_state().unsaved_buffers,
            open_buffers: host.open_buffer_count(),
            pending_wait_requests: host.protected_state().pending_wait_requests,
            live_terminals: host.protected_state().live_terminals,
            terminal_sessions: host.app().terminals.len(),
        }),
        ClientRequest::SessionPreview => Ok(HostResponse::SessionPreview {
            preview: host.session_preview().into(),
        }),
        ClientRequest::Invoke { command } => {
            if !allow_invoke {
                Err(anyhow::anyhow!(
                    "semantic commands require the attached interactive client"
                ))
            } else {
                parse_named_command(&command.name, command.argument.as_deref())
                    .map_err(anyhow::Error::from)
                    .and_then(|invocation| {
                        host.execute_expected_command(
                            command.frame.into(),
                            command.buffer.into(),
                            command.revision.into(),
                            invocation,
                        )
                        .map_err(anyhow::Error::from)
                    })
                    .map(|outcome| HostResponse::CommandResult {
                        outcome: outcome.into(),
                    })
            }
        }
        ClientRequest::ListBuffers => Ok(HostResponse::Buffers {
            buffers: host.buffer_metadata().into_iter().map(Into::into).collect(),
        }),
        ClientRequest::ReadBuffer { buffer } => host
            .read_buffer(buffer.into())
            .map(|buffer| HostResponse::Buffer {
                buffer: buffer.into(),
            })
            .map_err(anyhow::Error::from),
        ClientRequest::OpenBuffers { paths, activate } => {
            if paths.is_empty() || paths.len() > 32 {
                Err(anyhow::anyhow!("open request requires 1 to 32 paths"))
            } else {
                host.open_buffers(paths.into_iter().map(decode_path), activate)
                    .map(|buffers| HostResponse::Opened {
                        buffers: buffers.into_iter().map(Into::into).collect(),
                    })
            }
        }
        ClientRequest::ApplyTransaction {
            buffer,
            expected,
            changes,
        } => {
            if changes.is_empty() || changes.len() > 4096 {
                Err(anyhow::anyhow!("transaction requires 1 to 4096 changes"))
            } else if changes.iter().any(|change| change.from > change.to) {
                Err(anyhow::anyhow!(
                    "transaction ranges must be forward and half-open"
                ))
            } else {
                let transaction = Transaction::new(
                    changes
                        .into_iter()
                        .map(|TransportChange { from, to, text }| Change::new(from, to, text))
                        .collect(),
                );
                match host.apply_expected_transaction(buffer.into(), expected.into(), transaction) {
                    Ok(revision) => Ok(HostResponse::TransactionApplied {
                        buffer,
                        revision: revision.into(),
                    }),
                    Err(BufferRequestError::Stale { expected, actual }) => {
                        Ok(HostResponse::StaleRevision {
                            buffer,
                            expected: expected.into(),
                            actual: actual.into(),
                        })
                    }
                    Err(error) => Err(anyhow::Error::from(error)),
                }
            }
        }
        ClientRequest::SaveBuffer { buffer } => {
            host.save_buffer(buffer.into())
                .map(|revision| HostResponse::Saved {
                    buffer,
                    revision: revision.into(),
                })
        }
        ClientRequest::CloseBuffer { buffer, discard } => host
            .close_buffer(buffer.into(), discard)
            .map(|()| HostResponse::Closed { buffer }),
        ClientRequest::CreateWait { paths } => {
            if paths.is_empty() || paths.len() > 32 {
                Err(anyhow::anyhow!("wait request requires 1 to 32 paths"))
            } else {
                host.create_wait_request(paths.into_iter().map(decode_path), true)
                    .map(|(token, buffers)| HostResponse::WaitCreated {
                        token: token.into(),
                        buffers: buffers.into_iter().map(Into::into).collect(),
                        interactive_attached,
                    })
            }
        }
        ClientRequest::WaitStatus { token } => host
            .wait_status(token.into())
            .map(|status| HostResponse::WaitState {
                token,
                status: status.into(),
                interactive_attached,
            })
            .ok_or_else(|| anyhow::anyhow!("unknown wait token {token}")),
        ClientRequest::CompleteWaitBuffer { token, buffer } => host
            .complete_wait_buffer(token.into(), buffer.into())
            .and_then(|()| {
                host.wait_status(token.into())
                    .ok_or_else(|| anyhow::anyhow!("unknown wait token {token}"))
            })
            .map(|status| HostResponse::WaitState {
                token,
                status: status.into(),
                interactive_attached,
            }),
        ClientRequest::CancelWait { token } => host
            .cancel_wait(token.into(), "wait client cancelled the request")
            .and_then(|()| {
                host.wait_status(token.into())
                    .ok_or_else(|| anyhow::anyhow!("unknown wait token {token}"))
            })
            .map(|status| HostResponse::WaitState {
                token,
                status: status.into(),
                interactive_attached,
            }),
        _ => return None,
    };
    let response = result.unwrap_or_else(|error| HostResponse::Error {
        message: error.to_string(),
    });
    let publish_frame = workspace_response_publishes_frame(&response);
    Some(WorkspaceReply {
        response,
        publish_frame,
    })
}

#[cfg(unix)]
fn send_control_response(
    controls: &mut std::collections::HashMap<u64, runyte::workspace::transport::ResponseSender>,
    id: u64,
    response: HostResponse,
) {
    if controls
        .get(&id)
        .is_none_or(|responses| responses.try_send(response).is_err())
    {
        controls.remove(&id);
    }
}

#[cfg(unix)]
fn send_active_response(active: &mut Option<AttachedClient>, response: HostResponse) {
    let Some(client) = active.as_ref() else {
        *active = None;
        return;
    };
    // Distinguish a client that is behind from one that is gone. Only the
    // latter ends the attachment; treating momentary backpressure as a
    // disconnect closed live sessions during bursts of frames.
    if let Err(error) = client.responses.try_send(response) {
        let connection = client.id;
        match error {
            tokio::sync::mpsc::error::TrySendError::Closed(_) => {
                // Recorded here, where the failing write observed it. The
                // `Disconnected` event that follows finds no attachment and
                // stays quiet rather than reporting the same departure twice.
                log_info!(
                    "client",
                    "interactive client disconnected";
                    "connection" => connection,
                    "observed" => "control response"
                );
                *active = None;
            }
            // A frame is a whole snapshot, so skipping one costs nothing: the
            // next publish supersedes it. Anything else carries state the
            // client cannot reconstruct, and a channel this full means it is
            // not draining at all, so detaching says so rather than losing a
            // control message in silence.
            tokio::sync::mpsc::error::TrySendError::Full(HostResponse::Frame { .. }) => {}
            tokio::sync::mpsc::error::TrySendError::Full(_) => {
                // The client is still connected, so no `Disconnected` event
                // will follow: this is the only record a stalled reader
                // produces.
                log_warn!(
                    "client",
                    "interactive client stopped reading; ending the attachment";
                    "connection" => connection
                );
                *active = None;
            }
        }
    }
}

#[cfg(unix)]
fn detach_client(active: &mut Option<AttachedClient>, directory: Option<&Path>) {
    if let Some(client) = active.take() {
        let _ = client.responses.try_send(HostResponse::Detached {
            directory_bytes: directory.map(encode_path),
        });
    }
}

#[cfg(unix)]
fn complete_attached_waits(host: &mut WorkspaceHost, active: &mut Option<AttachedClient>) {
    let tokens = active
        .as_ref()
        .map(|client| client.wait_tokens.clone())
        .unwrap_or_default();
    for token in tokens {
        let status = match host.complete_wait_request(token.into()) {
            Ok(()) => host
                .wait_status(token.into())
                .expect("completed wait exists"),
            Err(error) => {
                let _ = host.cancel_wait(
                    token.into(),
                    format!("attached TUI quit before successful wait completion: {error}"),
                );
                host.wait_status(token.into())
                    .expect("cancelled wait exists")
            }
        };
        send_active_response(
            active,
            HostResponse::WaitState {
                token,
                status: status.into(),
                interactive_attached: false,
            },
        );
    }
}

#[cfg(unix)]
fn finish_attached_detach(host: &mut WorkspaceHost, active: &mut Option<AttachedClient>) {
    complete_attached_waits(host, active);
    detach_client(active, None);
}

/// Ends a persistent session in response to an editor-level quit.
///
/// The app owns the immediate dirty-buffer and terminal guards. Recheck the
/// host-wide answer after completing this client's wait requests so a control
/// client that raced the command cannot be abandoned. A force spelling may
/// discard unsaved buffers, but it still cannot end terminal children or
/// another caller's pending wait.
#[cfg(unix)]
fn finish_attached_quit(
    host: &mut WorkspaceHost,
    active: &mut Option<AttachedClient>,
    force: bool,
) -> bool {
    complete_attached_waits(host, active);
    let mut protected = host.protected_state();
    if force {
        protected.unsaved_buffers = 0;
    }
    if !protected.is_empty() {
        host.report_host_error(format!(
            "cannot quit persistent session: {}; finish or close that state, or use :detach to leave the session running",
            protected.refusal()
        ));
        return false;
    }

    // Keep the response sender in `active` after queuing the terminal reply.
    // `flush_connections` needs both the sender and connection identity to
    // keep the runtime alive until the reply is written. Taking it here lets a
    // fast shutdown end the process with a completed wait or `ShuttingDown`
    // message still in flight, which is most visible on macOS.
    //
    // `:quit-here` still carries the selected directory through a detach-shaped
    // response because the shell handoff belongs to the client. That response
    // does not keep the host alive: the caller marks it for shutdown as soon as
    // this function succeeds.
    let directory = host.quit_directory().map(Path::to_path_buf);
    if let Some(client) = active.as_ref() {
        let response = directory
            .as_ref()
            .map_or(HostResponse::ShuttingDown, |directory| {
                HostResponse::Detached {
                    directory_bytes: Some(encode_path(directory)),
                }
            });
        let _ = client.responses.try_send(response);
    }
    true
}

#[cfg(unix)]
fn switch_attached_workspace(
    host: &mut WorkspaceHost,
    active: &mut Option<AttachedClient>,
    request: runyte::app::WorkspaceSwitchRequest,
) {
    let tokens = active
        .as_ref()
        .map(|client| client.wait_tokens.clone())
        .unwrap_or_default();
    for token in tokens {
        let _ = host.cancel_wait(token.into(), "TUI switched to another workspace");
    }
    send_active_response(
        active,
        HostResponse::SwitchWorkspace {
            selector_bytes: encode_path(&request.selector),
            working_directory_bytes: encode_path(&request.working_directory),
        },
    );
    *active = None;
}

/// Reads the terminal's current shape.
///
/// Must be called before an `EventStream` exists: Crossterm falls back to a
/// cursor-position query when `TIOCGWINSZ` is unavailable, and an event reader
/// would consume the terminal's answer.
#[cfg(unix)]
fn current_frame_geometry() -> Result<runyte::app::FrameGeometry> {
    let (width, height) = crossterm::terminal::size()?;
    Ok(ui::frame_geometry(ratatui::layout::Rect::new(
        0, 0, width, height,
    )))
}

/// Attaches, and keeps attaching wherever the editor asks to go next.
///
/// One process for the whole session. The previous arrangement replaced the
/// re-exec by spawning a child `runyte --persistent` and blocking on it, so moving
/// from one workspace to another and back again stacked processes and quitting
/// unwound a stack.
#[cfg(unix)]
async fn run_workspace_switcher(
    endpoint: LocalEndpoint,
    mouse_enabled: bool,
    cwd_file: Option<&Path>,
    config: &Config,
    config_path: Option<&Path>,
) -> Result<()> {
    let mut termination = TerminationSignals::new()?;
    let _terminal = TerminalGuard::enter(mouse_enabled)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout()))?;
    // Probe the terminal before the event stream exists. Where `TIOCGWINSZ` is
    // unavailable Crossterm falls back to asking the terminal for its cursor
    // position, and an event reader would consume the reply.
    let mut geometry = current_frame_geometry()?;
    let mut terminal_events = EventStream::new();
    let mut current = endpoint;
    // Where to fall back to when a destination turns out to be unreachable.
    let mut previous: Option<LocalEndpoint> = None;
    let mut notice: Option<String> = None;
    loop {
        let attachment = tokio::select! {
            attachment = run_attached(
                &current,
                &mut terminal,
                &mut terminal_events,
                &mut geometry,
                None,
                cwd_file,
                notice.take(),
            ) => attachment,
            signal = termination.recv() => return Err(terminated(signal)),
        };
        let Some(outcome) =
            recover_switched_attachment(attachment, &mut current, &mut previous, &mut notice)?
        else {
            continue;
        };
        match outcome {
            AttachOutcome::Detached => return Ok(()),
            AttachOutcome::Switch {
                selector,
                working_directory,
            } => {
                match prepare_switch_target(
                    &selector,
                    &working_directory,
                    &current,
                    config,
                    config_path,
                )
                .await
                {
                    Ok(Some(next)) => {
                        previous = Some(std::mem::replace(&mut current, next));
                    }
                    // Already attached here; the editor asked for the workspace
                    // it is in, so there is nothing to move to.
                    Ok(None) => {}
                    Err(error) => notice = Some(format!("{error:#}")),
                }
            }
            AttachOutcome::Refused(message) => match previous.take() {
                // A destination we reached for is busy. Go back where we were
                // and say so, rather than ending the session.
                Some(source) => {
                    current = source;
                    notice = Some(message);
                }
                // Refused on the very first attachment: there is nowhere to
                // return to, so this is the ordinary attach failure.
                None => anyhow::bail!(message),
            },
        }
    }
}

/// Returns a successful attachment, or restores the source after a failed
/// switched attachment. The first attachment has no safe recovery target and
/// therefore preserves its ordinary error behavior.
#[cfg(unix)]
fn recover_switched_attachment<T>(
    attachment: Result<T>,
    current: &mut LocalEndpoint,
    previous: &mut Option<LocalEndpoint>,
    notice: &mut Option<String>,
) -> Result<Option<T>> {
    match attachment {
        Ok(outcome) => Ok(Some(outcome)),
        Err(error) => match previous.take() {
            // A destination may disappear, reject our protocol, or fail
            // during its handshake. Once switching is an editor action,
            // those failures belong on the source workspace's status line
            // rather than terminating the person's TUI.
            Some(source) => {
                *current = source;
                *notice = Some(format!("{error:#}"));
                Ok(None)
            }
            None => Err(error),
        },
    }
}

/// Resolves where a switch should attach, starting a host when none is running.
///
/// Returns `Ok(None)` when the destination is the workspace already attached.
/// The client has never had to do this before: it used to hand a directory to a
/// child process and let that child rediscover everything.
#[cfg(unix)]
async fn prepare_switch_target(
    selector: &Path,
    working_directory: &Path,
    current: &LocalEndpoint,
    config: &Config,
    config_path: Option<&Path>,
) -> Result<Option<LocalEndpoint>> {
    if let Ok(host) = resolve_registered_host_from_directory(selector, working_directory) {
        if host.project_root == current.project_root() {
            return Ok(None);
        }
        return Ok(Some(host.endpoint().clone()));
    }
    let requested = resolve_known_workspace_from_directory(
        selector,
        working_directory,
        &config.workspace.state,
    )
    .await?
    .unwrap_or_else(|| workspace_selector_path(selector, working_directory));
    let mut reserved_user_roots = config_path
        .map(|path| config::config_root_for(path, working_directory))
        .into_iter()
        .collect::<Vec<_>>();
    if let Some(cache_root) = external_open::cache_root() {
        reserved_user_roots.push(cache_root);
    }
    let requested = initialize_attached_directory(
        &requested,
        selector,
        &config.workspace.state,
        &reserved_user_roots,
    )?;
    let startup =
        HostStartup::new(std::env::current_exe()?, "destination").with_config(config_path);
    let state_root = project_root::resolve_state_root(&requested, &config.workspace.state);
    let endpoint = LocalEndpoint::discover(&state_root, &requested)?;
    if endpoint.project_root() == current.project_root() {
        return Ok(None);
    }
    if let Err(error) = connect_control(&endpoint).await {
        if error.downcast_ref::<IncompatibleHost>().is_some() {
            return Err(error);
        }
        start_detached_host(&endpoint, startup).await?;
    }
    Ok(Some(endpoint))
}

/// Attaches a terminal for the lifetime of one `--wait` request.
///
/// A wait request never moves between workspaces, so it owns its terminal for a
/// single attachment instead of going through the switcher.
#[cfg(unix)]
async fn attach_for_wait(
    endpoint: &LocalEndpoint,
    mouse_enabled: bool,
    token: WaitToken,
    control: &mut LocalClient,
    termination: &mut TerminationSignals,
) -> Result<()> {
    let _terminal = TerminalGuard::enter(mouse_enabled)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout()))?;
    let mut geometry = current_frame_geometry()?;
    let mut terminal_events = EventStream::new();
    let attachment = tokio::select! {
        attachment = run_attached(
            endpoint,
            &mut terminal,
            &mut terminal_events,
            &mut geometry,
            Some(token),
            None,
            None,
        ) => attachment,
        signal = termination.recv() => return Err(terminated(signal)),
    };
    match attachment {
        Err(attachment_error) => {
            // Completing a wait closes its interactive attachment after
            // queuing the terminal state. A periodic status poll can race that
            // close and see BrokenPipe before the queued completion reaches
            // the reader. The wait request is durable host state, so resolve
            // that race through the independent control connection instead of
            // turning a successful edit into a failed caller process.
            control.send(&ClientRequest::WaitStatus { token }).await?;
            match control.recv().await? {
                Some(HostResponse::WaitState {
                    token: response_token,
                    status: WaitStatus::Completed,
                    ..
                }) if response_token == token => Ok(()),
                Some(HostResponse::WaitState {
                    token: response_token,
                    status: WaitStatus::Cancelled { reason },
                    ..
                }) if response_token == token => anyhow::bail!(reason),
                Some(HostResponse::WaitState {
                    token: response_token,
                    status: WaitStatus::Pending { .. },
                    ..
                }) if response_token == token => Err(attachment_error
                    .context("wait attachment failed while the request remained pending")),
                Some(response) => Err(attachment_error.context(format!(
                    "wait attachment failed and status recovery returned {response:?}"
                ))),
                None => Err(attachment_error
                    .context("wait attachment failed and its host disconnected during recovery")),
            }
        }
        Ok(AttachOutcome::Detached) => Ok(()),
        Ok(AttachOutcome::Switch { .. }) => {
            anyhow::bail!("wait request cannot switch workspaces")
        }
        Ok(AttachOutcome::Refused(message)) => anyhow::bail!(message),
    }
}

/// How one attachment ended, so the switcher can decide what to do next.
#[cfg(unix)]
enum AttachOutcome {
    /// The person is finished with this client.
    Detached,
    /// The editor asked to move to another workspace.
    Switch {
        selector: std::path::PathBuf,
        working_directory: std::path::PathBuf,
    },
    /// The destination already has an interactive TUI. Routine once switching is
    /// a keystroke, so it is an outcome rather than a failure.
    Refused(String),
}

/// Records both ends of one successfully established interactive attachment.
///
/// Created only after the host accepts the handshake. Drop covers ordinary
/// detach, switching, transport errors, and cancellation of `run_attached` by
/// a termination signal, so no exit path can leave a long attachment looking
/// idle since its arrival.
#[cfg(unix)]
struct AttachedWorkspaceActivity {
    project_root: PathBuf,
    record: fn(&Path) -> Result<()>,
}

#[cfg(unix)]
impl AttachedWorkspaceActivity {
    fn begin(project_root: &Path) -> Self {
        Self::begin_with(project_root, record_workspace_activity)
    }

    fn begin_with(project_root: &Path, record: fn(&Path) -> Result<()>) -> Self {
        let _ = record(project_root);
        Self {
            project_root: project_root.to_path_buf(),
            record,
        }
    }
}

#[cfg(unix)]
impl Drop for AttachedWorkspaceActivity {
    fn drop(&mut self) {
        let _ = (self.record)(&self.project_root);
    }
}

/// Runs one attachment to completion, drawing into a terminal it does not own.
///
/// The caller keeps the terminal and the event stream across attachments:
/// leaving and re-entering the alternate screen on every switch would flash, and
/// Crossterm's reader is process-global, so churning event streams around a
/// reconnect can lose a partially buffered escape sequence.
#[cfg(unix)]
async fn run_attached(
    endpoint: &LocalEndpoint,
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    terminal_events: &mut EventStream,
    geometry: &mut runyte::app::FrameGeometry,
    wait_token: Option<WaitToken>,
    cwd_file: Option<&Path>,
    notice: Option<String>,
) -> Result<AttachOutcome> {
    let mut client =
        LocalClient::connect_with_handoff(endpoint, *geometry, true, cwd_file.is_some()).await?;
    match client.recv().await? {
        Some(response @ HostResponse::Welcome { .. }) => {
            validate_welcome(&response, true).map_err(anyhow::Error::msg)?;
        }
        Some(HostResponse::Refused { message }) => return Ok(AttachOutcome::Refused(message)),
        Some(response) => anyhow::bail!("unexpected workspace handshake response: {response:?}"),
        None => anyhow::bail!("workspace host disconnected during handshake"),
    }
    let _activity = AttachedWorkspaceActivity::begin(endpoint.project_root());
    if let Some(message) = notice {
        client.send(&ClientRequest::Notify { message }).await?;
    }
    // Ratatui diffs against its previous buffer, which starts empty for a new
    // terminal and holds the previous workspace's frame for a reused one. Either
    // way the cells this frame leaves blank would not be emitted, so the screen
    // has to be cleared before the first draw of each attachment.
    //
    // `Terminal::clear` is the obvious call and the wrong one: it asks the
    // terminal for its cursor position so it can restore it, and the event
    // stream this client is already running would consume the reply. Resizing to
    // the size we already know clears the screen and resets the back buffer
    // without asking the terminal anything.
    terminal.resize(ratatui::layout::Rect::new(
        0,
        0,
        geometry.screen.width,
        geometry.screen.height,
    ))?;
    let mut current_frame = match client.recv().await? {
        Some(HostResponse::Frame { frame }) => (*frame)
            .try_into()
            .map_err(|error: String| anyhow::anyhow!(error))?,
        Some(response) => anyhow::bail!("workspace host sent no initial frame: {response:?}"),
        None => anyhow::bail!("workspace host disconnected before its initial frame"),
    };
    if let Some(token) = wait_token {
        client.send(&ClientRequest::AttachWait { token }).await?;
        loop {
            match client.recv().await? {
                Some(HostResponse::Frame { frame }) => {
                    current_frame = (*frame)
                        .try_into()
                        .map_err(|error: String| anyhow::anyhow!(error))?;
                    terminal.draw(|frame| ui::render_host_frame(frame, &current_frame))?;
                }
                Some(HostResponse::TerminalDamage { damage }) => {
                    if apply_terminal_damage(&mut current_frame, &damage)? {
                        terminal.draw(|frame| ui::render_host_frame(frame, &current_frame))?;
                    } else {
                        client.send(&ClientRequest::Resynchronize).await?;
                    }
                }
                Some(HostResponse::WaitState {
                    token: response_token,
                    status: WaitStatus::Pending { .. },
                    ..
                }) if response_token == token => break,
                Some(HostResponse::WaitState {
                    token: response_token,
                    status: WaitStatus::Completed,
                    ..
                }) if response_token == token => return Ok(AttachOutcome::Detached),
                Some(HostResponse::WaitState {
                    token: response_token,
                    status: WaitStatus::Cancelled { reason },
                    ..
                }) if response_token == token => anyhow::bail!(reason),
                Some(HostResponse::Error { message } | HostResponse::Refused { message }) => {
                    anyhow::bail!(message)
                }
                Some(HostResponse::Detached { .. } | HostResponse::ShuttingDown) | None => {
                    anyhow::bail!("workspace host disconnected while attaching wait request")
                }
                Some(_) => {}
            }
        }
    }
    terminal.draw(|frame| ui::render_host_frame(frame, &current_frame))?;
    let mut key_repeat_detector = KeyRepeatDetector::default();
    let mut pointer_batcher = PointerBatcher::default();
    let mut pointer_tick = tokio::time::interval(Duration::from_millis(8));
    pointer_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut wait_tick = tokio::time::interval(Duration::from_millis(100));
    wait_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            input = terminal_events.next() => {
                let Some(event) = input.transpose()? else {
                    if let Some(batch) = pointer_batcher.take() {
                        client.send(&batch.request()).await?;
                    }
                    let _ = client.send(&ClientRequest::Detach).await;
                    anyhow::ensure!(
                        wait_token.is_none(),
                        "wait request lost its terminal before completion"
                    );
                    break;
                };
                if let CrosstermEvent::Resize(width, height) = event {
                    key_repeat_detector.observe(None, None, Instant::now());
                    if let Some(batch) = pointer_batcher.take() {
                        client.send(&batch.request()).await?;
                    }
                    *geometry = ui::frame_geometry(ratatui::layout::Rect::new(0, 0, width, height));
                    client
                        .send(&ClientRequest::Resize {
                            geometry: (*geometry).into(),
                        })
                        .await?;
                    continue;
                }
                let key_kind = terminal_key_kind(&event);
                let Some(input) = convert_event(event)? else {
                    key_repeat_detector.observe(key_kind, None, Instant::now());
                    continue;
                };
                let repeated = key_repeat_detector.observe(key_kind, Some(&input), Instant::now());
                if let Some(message) = rejected_text_input(&input) {
                    if let Some(batch) = pointer_batcher.take() {
                        client.send(&batch.request()).await?;
                    }
                    client.send(&ClientRequest::Notify { message }).await?;
                    continue;
                }
                if is_passive_pointer(&input) {
                    continue;
                }
                match input {
                    InputEvent::Pointer(event) if is_wheel_event(event.kind) => {
                        if let Some(batch) = pointer_batcher.push_wheel(event, current_frame.id) {
                            client.send(&batch.request()).await?;
                        }
                    }
                    InputEvent::Pointer(event) => {
                        if let Some(batch) = pointer_batcher.take() {
                            client.send(&batch.request()).await?;
                        }
                        client.send(&ClientRequest::Pointer {
                            event: event.into(),
                            frame: current_frame.id.into(),
                            repetitions: 1,
                        }).await?;
                    }
                    event => {
                        if let Some(batch) = pointer_batcher.take() {
                            client.send(&batch.request()).await?;
                        }
                        client
                            .send(&ClientRequest::Input {
                                event: event.into(),
                                repeated,
                            })
                            .await?
                    }
                }
            }
            _ = pointer_tick.tick(), if pointer_batcher.pending.is_some() => {
                if let Some(batch) = pointer_batcher.take() {
                    client.send(&batch.request()).await?;
                }
            }
            response = client.recv() => {
                match response? {
                    Some(HostResponse::Frame { frame }) => {
                        current_frame = (*frame)
                            .try_into()
                            .map_err(|error: String| anyhow::anyhow!(error))?;
                        terminal.draw(|frame| ui::render_host_frame(frame, &current_frame))?;
                    }
                    Some(HostResponse::TerminalDamage { damage }) => {
                        if apply_terminal_damage(&mut current_frame, &damage)? {
                            terminal.draw(|frame| ui::render_host_frame(frame, &current_frame))?;
                        } else {
                            client.send(&ClientRequest::Resynchronize).await?;
                        }
                    }
                    Some(HostResponse::WaitState { token, status, .. }) if Some(token) == wait_token => {
                        match status {
                            WaitStatus::Completed => break,
                            WaitStatus::Cancelled { reason } => anyhow::bail!(reason),
                            WaitStatus::Pending { .. } => {}
                        }
                    }
                    Some(HostResponse::Detached { directory_bytes }) => {
                        anyhow::ensure!(wait_token.is_none(), "wait request ended before completion");
                        // `:quit-here` chose this directory inside the host. The
                        // file belongs to this process, so writing it is the
                        // client's half of the handoff.
                        if let (Some(cwd_file), Some(directory)) =
                            (cwd_file, directory_bytes.map(decode_path))
                        {
                            write_cwd_file(cwd_file, &directory)?;
                        }
                        break;
                    }
                    Some(HostResponse::ShuttingDown) | None => {
                        anyhow::ensure!(wait_token.is_none(), "wait request ended before completion");
                        break;
                    }
                    Some(HostResponse::SwitchWorkspace {
                        selector_bytes,
                        working_directory_bytes,
                    }) => {
                        anyhow::ensure!(
                            wait_token.is_none(),
                            "wait request was cancelled by a workspace switch"
                        );
                        return Ok(AttachOutcome::Switch {
                            selector: decode_path(selector_bytes),
                            working_directory: decode_path(working_directory_bytes),
                        });
                    }
                    Some(HostResponse::Refused { message } | HostResponse::Error { message }) => {
                        anyhow::bail!(message);
                    }
                    Some(HostResponse::Welcome { .. }) => {}
                    Some(_) => {}
                }
            }
            _ = wait_tick.tick(), if wait_token.is_some() => {
                client.send(&ClientRequest::WaitStatus {
                    token: wait_token.expect("guarded by is_some"),
                }).await?;
            }
        }
    }
    Ok(AttachOutcome::Detached)
}

#[cfg(unix)]
fn apply_terminal_damage(
    current: &mut runyte::workspace::HostFrame,
    damage: &runyte::protocol::TerminalDamageFrame,
) -> Result<bool> {
    let mut wire: runyte::protocol::HostFrame = current.clone().into();
    if !damage.apply(&mut wire) {
        return Ok(false);
    }
    *current = wire
        .try_into()
        .map_err(|error: String| anyhow::anyhow!(error))?;
    Ok(true)
}

#[cfg(unix)]
async fn run_wait(
    endpoint: LocalEndpoint,
    targets: Vec<LaunchTarget>,
    config_path: Option<std::path::PathBuf>,
    mouse_enabled: bool,
    verbosity: u8,
    log: Option<&Path>,
) -> Result<()> {
    // Install before the durable request is created. A signal received while
    // its response is in flight is retained until the token is known, then
    // follows the same explicit cancellation path as every other error.
    let mut termination = TerminationSignals::new()?;
    let caller_directory = std::env::current_dir()?;
    let paths = targets
        .into_iter()
        .map(|target| {
            if target.path.is_absolute() {
                target.path
            } else {
                caller_directory.join(target.path)
            }
        })
        .collect::<Vec<_>>();
    let mut control = match connect_control(&endpoint).await {
        Ok(client) => {
            // The host was already there, so this launch did not choose its
            // logging and must not appear to have.
            report_retained_logging(verbosity, log);
            client
        }
        // A host of another version is still holding this workspace. Starting a
        // second one would only fail to bind, and displacing it silently is not
        // this command's decision to make, so the error names the process and
        // the command that ends it. Its endpoint left behind after it exits is
        // a different case and reads as stale, so it falls through and is
        // replaced like any other one.
        Err(error) if error.downcast_ref::<IncompatibleHost>().is_some() => return Err(error),
        Err(_) => {
            let startup = HostStartup::new(std::env::current_exe()?, "--wait")
                .with_working_directory(&caller_directory)
                .with_config(config_path.as_deref())
                .with_logging(verbosity, log)
                .with_targets(paths.clone());
            start_detached_host(&endpoint, startup).await?;
            connect_control(&endpoint)
                .await
                .context("workspace host for --wait did not publish an endpoint")?
        }
    };
    control
        .send(&ClientRequest::CreateWait {
            paths: paths.iter().map(|path| encode_path(path)).collect(),
        })
        .await?;
    let (token, interactive_attached) = match control.recv().await? {
        Some(HostResponse::WaitCreated {
            token,
            interactive_attached,
            ..
        }) => (token, interactive_attached),
        Some(HostResponse::Error { message } | HostResponse::Refused { message }) => {
            anyhow::bail!(message)
        }
        Some(response) => anyhow::bail!("unexpected wait-create response: {response:?}"),
        None => anyhow::bail!("workspace host disconnected while creating wait request"),
    };

    let outcome = if let Some(signal) = termination.received() {
        Err(terminated(signal))
    } else if interactive_attached {
        wait_for_completion(
            &mut control,
            &endpoint,
            mouse_enabled,
            token,
            &mut termination,
        )
        .await
    } else {
        attach_for_wait(
            &endpoint,
            mouse_enabled,
            token,
            &mut control,
            &mut termination,
        )
        .await
    };
    if outcome.is_err() {
        let _ = control.send(&ClientRequest::CancelWait { token }).await;
    }
    outcome
}

#[cfg(unix)]
async fn list_sessions(state: &Path, all_namespaces: bool) -> Result<()> {
    let workspaces = if all_namespaces {
        known_workspaces_all_namespaces(state).await?
    } else {
        known_workspaces(state).await?
    };
    let width = abbreviated_id_width(workspaces.iter().map(|workspace| workspace.id.as_str()));
    let mut rows = workspaces
        .iter()
        .map(|workspace| {
            [
                workspace.id[..width.min(workspace.id.len())].to_owned(),
                workspace.name.clone().unwrap_or_else(|| "-".to_owned()),
                workspace.project_root.display().to_string(),
                workspace.state_label(),
                workspace
                    .unsaved_buffers
                    .map_or_else(String::new, |count| count.to_string()),
                workspace
                    .live_terminals
                    .map_or_else(String::new, |count| count.to_string()),
                workspace
                    .pending_wait_requests
                    .map_or_else(String::new, |count| count.to_string()),
                workspace
                    .interactive_attached
                    .map_or_else(String::new, |attached| {
                        if attached { "yes" } else { "no" }.to_owned()
                    }),
            ]
        })
        .collect::<Vec<_>>();
    let headings = [
        "ID".to_owned(),
        "NAME".to_owned(),
        "DIRECTORY".to_owned(),
        "STATE".to_owned(),
        "UNSAVED".to_owned(),
        "TERMINALS".to_owned(),
        "WAITING".to_owned(),
        "TUI".to_owned(),
    ];
    let mut widths = [0_usize; 8];
    for row in std::iter::once(&headings).chain(rows.iter()) {
        for (index, value) in row.iter().enumerate() {
            widths[index] =
                widths[index].max(unicode_width::UnicodeWidthStr::width(value.as_str()));
        }
    }
    print_workspace_row(&headings, &widths);
    print_workspace_row(&widths.map(|width| "-".repeat(width)), &widths);
    for row in rows.drain(..) {
        print_workspace_row(&row, &widths);
    }
    Ok(())
}

#[cfg(unix)]
fn print_workspace_row(row: &[String; 8], widths: &[usize; 8]) {
    let cells = std::array::from_fn::<_, 8, _>(|index| pad_table_cell(&row[index], widths[index]));
    println!(
        "{}  {}  {}  {}  {}  {}  {}  {}",
        cells[0], cells[1], cells[2], cells[3], cells[4], cells[5], cells[6], cells[7]
    );
}

#[cfg(unix)]
fn pad_table_cell(value: &str, width: usize) -> String {
    let used = unicode_width::UnicodeWidthStr::width(value);
    format!("{value}{}", " ".repeat(width.saturating_sub(used)))
}

#[cfg(unix)]
async fn start_selected_session(
    selector: &Path,
    config_path: Option<&Path>,
    verbosity: u8,
    log: Option<&Path>,
) -> Result<()> {
    let (config, config_path) = Config::load(config_path)?;
    let working_directory = std::env::current_dir()?;
    let requested = resolve_known_workspace_from_directory(
        selector,
        &working_directory,
        &config.workspace.state,
    )
    .await?
    .unwrap_or_else(|| {
        if selector.is_absolute() {
            selector.to_path_buf()
        } else {
            working_directory.join(selector)
        }
    });
    let startup = HostStartup::new(std::env::current_exe()?, "started")
        .with_config(config_path.as_deref())
        .with_logging(verbosity, log);
    let endpoint =
        resolve_workspace_endpoint(&requested, &config.workspace.state, config_path.as_deref())?;
    report_retained_logging_if_running(&endpoint, verbosity, log).await;
    ensure_workspace_host(
        &requested,
        &config.workspace.state,
        config_path.as_deref(),
        startup,
    )
    .await
    .map(|_| ())
}

/// Renames a session whether or not it is running.
///
/// A stopped session has no endpoint to ask, so this cannot go through
/// [`resolve_lifecycle_endpoint`] like the other lifecycle commands: the name
/// of a stopped workspace lives in the visited history that lists it. The
/// catalog owns both halves of that choice, so `--session-rename` and the
/// editor's session list rename exactly the same set of sessions.
#[cfg(unix)]
async fn rename_selected_session(
    selector: &Path,
    name: &str,
    config_path: Option<&Path>,
) -> Result<()> {
    let (config, config_path) = Config::load(config_path)?;
    rename_known_workspace(
        selector,
        name,
        &config.workspace.state,
        config_path.as_deref(),
    )
    .await
}

/// Stops the selected host, preferring the request only it can answer.
///
/// A host that speaks this protocol refuses while it holds unsaved buffers, so
/// asking is always tried first. A host from another version cannot be asked at
/// all, and refusing there would leave the workspace unreachable for good:
/// nothing else can release the endpoint every client resolves to it.
#[cfg(unix)]
async fn stop_selected_session(endpoint: &LocalEndpoint, force: bool) -> Result<()> {
    if force {
        let Err(error) = force_shutdown_host(endpoint).await else {
            return Ok(());
        };
        if error.downcast_ref::<IncompatibleHost>().is_none() {
            return Err(error);
        }
        let host = terminate_incompatible_host(endpoint).await?;
        eprintln!(
            "force-stopped persistent session process {} (protocol {}); its protected live state was discarded",
            host.pid, host.protocol
        );
        return Ok(());
    }
    let Err(error) = shutdown_host(endpoint).await else {
        return Ok(());
    };
    if error.downcast_ref::<IncompatibleHost>().is_none() {
        return Err(error);
    }
    let host = endpoint
        .published_host()?
        .context("no workspace host is running there")?;
    anyhow::bail!(
        "persistent session process {} speaks incompatible protocol {}; it may own live terminals or unsaved buffers. Use a compatible client, or run --session-stop --force to terminate it",
        host.pid,
        host.protocol
    )
}

/// Tries every running host even when one refuses, so a protected session
/// cannot prevent unrelated clean sessions from stopping.
#[cfg(unix)]
async fn stop_all_sessions(
    state: &Path,
    config_path: Option<&Path>,
    force: bool,
    all_namespaces: bool,
) -> Result<()> {
    if all_namespaces {
        let hosts = registered_hosts_all_namespaces()?;
        let total = hosts.len();
        let mut stopped = 0;
        let mut failures = Vec::new();
        for host in hosts {
            match stop_selected_session(host.endpoint(), force).await {
                Ok(()) => stopped += 1,
                Err(error) => failures.push(format!(
                    "{} ({}): {error:#}",
                    host.name.as_deref().unwrap_or("unnamed"),
                    host.project_root.display()
                )),
            }
        }
        return report_stop_all(stopped, total, failures);
    }

    let running = known_workspaces(state)
        .await?
        .into_iter()
        .filter(|workspace| workspace.running)
        .collect::<Vec<_>>();
    let total = running.len();
    let mut stopped = 0;
    let mut failures = Vec::new();
    for workspace in running {
        let endpoint =
            resolve_registered_host(&workspace.project_root).map(|host| host.endpoint().clone());
        let result = match endpoint {
            Ok(endpoint) => stop_selected_session(&endpoint, force).await,
            Err(_) => {
                match resolve_lifecycle_endpoint(&workspace.project_root, state, config_path).await
                {
                    Ok(endpoint) => stop_selected_session(&endpoint, force).await,
                    Err(error) => Err(error),
                }
            }
        };
        match result {
            Ok(()) => stopped += 1,
            Err(error) => failures.push(format!(
                "{} ({}): {error:#}",
                workspace.display_name(),
                workspace.project_root.display()
            )),
        }
    }
    report_stop_all(stopped, total, failures)
}

#[cfg(unix)]
fn report_stop_all(stopped: usize, total: usize, failures: Vec<String>) -> Result<()> {
    if failures.is_empty() {
        println!(
            "stopped {stopped} session{}",
            if stopped == 1 { "" } else { "s" }
        );
        return Ok(());
    }
    anyhow::bail!(
        "stopped {stopped} of {total} running sessions; {} failed:\n{}",
        failures.len(),
        failures.join("\n")
    )
}

#[cfg(unix)]
async fn resolve_lifecycle_endpoint(
    selector: &Path,
    state: &Path,
    config_path: Option<&Path>,
) -> Result<LocalEndpoint> {
    if let Ok(host) = resolve_registered_host(selector) {
        return Ok(host.endpoint().clone());
    }

    let project_root = resolve_known_workspace(selector, state)
        .await?
        .unwrap_or_else(|| selector.to_path_buf());
    let endpoint = resolve_workspace_endpoint(&project_root, state, config_path)?;
    anyhow::ensure!(
        endpoint.metadata().exists() && endpoint.socket().exists(),
        "no running session matches {}; use --session-list to see available sessions",
        selector.display()
    );
    Ok(endpoint)
}

#[cfg(unix)]
async fn wait_for_completion(
    client: &mut LocalClient,
    endpoint: &LocalEndpoint,
    mouse_enabled: bool,
    token: WaitToken,
    termination: &mut TerminationSignals,
) -> Result<()> {
    loop {
        client.send(&ClientRequest::WaitStatus { token }).await?;
        let response = tokio::select! {
            response = client.recv() => response?,
            signal = termination.recv() => return Err(terminated(signal)),
        };
        match response {
            Some(HostResponse::WaitState {
                token: response_token,
                status,
                interactive_attached,
            }) if response_token == token => match status {
                WaitStatus::Completed => return Ok(()),
                WaitStatus::Cancelled { reason } => anyhow::bail!(reason),
                WaitStatus::Pending { .. } if !interactive_attached => {
                    return attach_for_wait(endpoint, mouse_enabled, token, client, termination)
                        .await;
                }
                WaitStatus::Pending { .. } => {}
            },
            Some(HostResponse::Error { message } | HostResponse::Refused { message }) => {
                anyhow::bail!(message)
            }
            Some(_) => {}
            None => anyhow::bail!("workspace host stopped before wait request completed"),
        }
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_millis(100)) => {}
            signal = termination.recv() => return Err(terminated(signal)),
        }
    }
}

struct HostServices {
    syntax_events: SyntaxEvents,
    git_events: Option<tokio::sync::mpsc::Receiver<GitServiceEvent>>,
    language_servers: LspHandle,
    lsp_events: tokio::sync::mpsc::Receiver<LspEvent>,
    file_picker_events: tokio::sync::mpsc::Receiver<runyte::file_picker::FilePickerEvent>,
    file_monitor: runyte::file_monitor::FileMonitorHandle,
    file_monitor_events: tokio::sync::mpsc::Receiver<runyte::buffer::FileObservationEvent>,
    workspace_events: Option<tokio::sync::mpsc::Receiver<HostEvent>>,
    /// Output from every child running on a terminal pane.
    ///
    /// Held here rather than inside the editor so a loop can wait on it beside
    /// its other sources without keeping the editor mutably borrowed for the
    /// whole of a `select!`.
    terminal_events: TerminalEvents,
}

async fn receive_workspace_event(
    events: &mut Option<tokio::sync::mpsc::Receiver<HostEvent>>,
) -> Option<HostEvent> {
    match events.as_mut() {
        Some(events) => events.recv().await,
        None => std::future::pending().await,
    }
}

fn start_host_services(
    app: &mut WorkspaceHost,
    startup: &mut StartupTrace,
    config_path: Option<&Path>,
) -> Result<HostServices> {
    let git_events = if let Some(provider) = GitCliProvider::from_environment() {
        let (service, events) = GitService::spawn(provider);
        app.attach_git_service(service);
        Some(events)
    } else {
        None
    };
    let (language_servers, lsp_events) =
        lsp::spawn(app.config.lsp.clone(), app.project_root.clone());
    startup.mark(StartupPhase::LspManagerSpawned);
    app.attach_lsp(language_servers.clone());
    let (syntax_worker, syntax_events) = syntax::spawn_background(Arc::clone(&app.registry));
    app.attach_syntax_worker(syntax_worker);
    let (file_scanner, file_picker_events) = file_picker::scanner();
    app.attach_file_scanner(file_scanner);
    let (file_monitor, file_monitor_events) = file_monitor::spawn();
    file_monitor.sync(app.file_monitor_requests());
    app.attach_word_index(word_index::spawn());
    #[cfg(unix)]
    let workspace_events = {
        let (service, mut events) = WorkspaceService::spawn(
            std::env::current_exe()?,
            app.config.workspace.state.clone(),
            config_path.map(Path::to_path_buf),
        );
        app.attach_workspace_service(service);
        let (host_events, receiver) = tokio::sync::mpsc::channel(16);
        tokio::spawn(async move {
            while let Some(event) = events.recv().await {
                if host_events.send(HostEvent::Workspace(event)).await.is_err() {
                    break;
                }
            }
        });
        Some(receiver)
    };
    #[cfg(not(unix))]
    let workspace_events = None;
    let terminal_events = app
        .take_terminal_events()
        .expect("terminal output is claimed once, when services start");
    Ok(HostServices {
        syntax_events,
        git_events,
        language_servers,
        lsp_events,
        file_picker_events,
        file_monitor,
        file_monitor_events,
        workspace_events,
        terminal_events,
    })
}

fn write_cwd_file(path: &Path, directory: &Path) -> Result<()> {
    let mut contents = directory.as_os_str().as_encoded_bytes().to_vec();
    #[cfg(unix)]
    contents.push(0);
    atomic_write_cwd_file(path, &contents)
        .with_context(|| format!("failed to write cwd file {}", path.display()))
}

#[cfg(unix)]
fn atomic_write_cwd_file(path: &Path, contents: &[u8]) -> io::Result<()> {
    static NEXT_TEMP: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    atomic_write_cwd_file_with(path, contents, || {
        NEXT_TEMP.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    })
}

#[cfg(unix)]
fn atomic_write_cwd_file_with(
    path: &Path,
    contents: &[u8],
    mut next_sequence: impl FnMut() -> u64,
) -> io::Result<()> {
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    path.file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "cwd file has no name"))?;
    for _ in 0..128 {
        let sequence = next_sequence();
        let temporary = parent.join(format!(".runyte-cwd-{}-{sequence}.tmp", std::process::id()));
        let mut file = match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temporary)
        {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        };
        let result = (|| {
            file.set_permissions(fs::Permissions::from_mode(0o600))?;
            file.write_all(contents)?;
            file.sync_all()?;
            drop(file);
            fs::rename(&temporary, path)
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        return result;
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "could not reserve a temporary cwd handoff file",
    ))
}

#[cfg(not(unix))]
fn atomic_write_cwd_file(path: &Path, contents: &[u8]) -> io::Result<()> {
    fs::write(path, contents)
}

fn is_passive_pointer(input: &InputEvent) -> bool {
    matches!(input, InputEvent::Pointer(event) if event.kind == PointerEventKind::Moved)
}

#[cfg(unix)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct PointerBatch {
    event: PointerEvent,
    frame: runyte::workspace::FrameId,
    repetitions: u16,
}

#[cfg(unix)]
impl PointerBatch {
    fn request(self) -> ClientRequest {
        ClientRequest::Pointer {
            event: self.event.into(),
            frame: self.frame.into(),
            repetitions: self.repetitions,
        }
    }
}

/// Coalesces only consecutive identical wheel reports. Clicks, drags, text,
/// and keys flush the pending run first so their ordering remains exact.
#[cfg(unix)]
#[derive(Debug, Default)]
struct PointerBatcher {
    pending: Option<PointerBatch>,
}

#[cfg(unix)]
impl PointerBatcher {
    fn push_wheel(
        &mut self,
        event: PointerEvent,
        frame: runyte::workspace::FrameId,
    ) -> Option<PointerBatch> {
        debug_assert!(is_wheel_event(event.kind));
        if let Some(pending) = self.pending.as_mut()
            && pending.event == event
            && pending.repetitions < MAX_POINTER_REPETITIONS
        {
            pending.frame = frame;
            pending.repetitions += 1;
            return None;
        }
        self.pending.replace(PointerBatch {
            event,
            frame,
            repetitions: 1,
        })
    }

    fn take(&mut self) -> Option<PointerBatch> {
        self.pending.take()
    }
}

fn is_wheel_event(kind: PointerEventKind) -> bool {
    matches!(
        kind,
        PointerEventKind::ScrollUp
            | PointerEventKind::ScrollDown
            | PointerEventKind::ScrollLeft
            | PointerEventKind::ScrollRight
    )
}

/// Reports terminal events that carry no editor input but still invalidate the
/// frame on screen.
///
/// `convert_event` yields nothing for a resize, so the input arm would skip
/// its draw and leave the previous shape rendered until the next key, command,
/// or Git refresh happened to redraw. The new size needs no editor state
/// change — Ratatui reconciles its buffers inside `draw`, and the layout reads
/// the new geometry from the frame — so the whole fix is to let the loop reach
/// that draw. Focus changes leave the shape alone and stay on the quiet path.
fn is_redraw_only_event(event: &CrosstermEvent) -> bool {
    match event {
        CrosstermEvent::Resize(_, _) => true,
        CrosstermEvent::FocusGained
        | CrosstermEvent::FocusLost
        | CrosstermEvent::Key(_)
        | CrosstermEvent::Mouse(_)
        | CrosstermEvent::Paste(_) => false,
    }
}

fn terminal_key_kind(event: &CrosstermEvent) -> Option<KeyEventKind> {
    match event {
        CrosstermEvent::Key(key) => Some(key.kind),
        _ => None,
    }
}

fn rejected_text_input(input: &InputEvent) -> Option<String> {
    let InputEvent::Text(text) = input else {
        return None;
    };
    (text.len() > runyte::input::MAX_TEXT_INPUT_BYTES).then(|| {
        format!(
            "text input exceeds the {} byte limit",
            runyte::input::MAX_TEXT_INPUT_BYTES
        )
    })
}

const MAX_LEGACY_REPEAT_INTERVAL: Duration = Duration::from_millis(250);
const MIN_LEGACY_INITIAL_DELAY: Duration = Duration::from_millis(180);

/// Identifies held keys in terminals that report every auto-repeat as a fresh
/// press instead of exposing `KeyEventKind::Repeat`.
///
/// A legacy repeat stream has a long initial delay followed by closely spaced
/// presses of the same key. Requiring both parts avoids treating ordinary fast
/// taps as held input. Enhanced terminal repeat and release events remain the
/// authoritative path when they are available.
#[derive(Default)]
struct KeyRepeatDetector {
    last_key: Option<KeyStroke>,
    last_press: Option<Instant>,
    previous_interval: Option<Duration>,
    legacy_repeat: bool,
}

impl KeyRepeatDetector {
    fn observe(
        &mut self,
        kind: Option<KeyEventKind>,
        input: Option<&InputEvent>,
        now: Instant,
    ) -> bool {
        match kind {
            Some(KeyEventKind::Release) => {
                self.reset();
                false
            }
            Some(KeyEventKind::Repeat) => matches!(input, Some(InputEvent::Key(_))),
            Some(KeyEventKind::Press) => {
                let Some(InputEvent::Key(key)) = input else {
                    self.reset();
                    return false;
                };
                self.observe_legacy_press(*key, now)
            }
            None => {
                self.reset();
                false
            }
        }
    }

    fn observe_legacy_press(&mut self, key: KeyStroke, now: Instant) -> bool {
        if self.last_key != Some(key) {
            self.last_key = Some(key);
            self.last_press = Some(now);
            self.previous_interval = None;
            self.legacy_repeat = false;
            return false;
        }

        let interval = self
            .last_press
            .map_or(Duration::MAX, |last| now.saturating_duration_since(last));
        let repeated = if self.legacy_repeat {
            interval <= MAX_LEGACY_REPEAT_INTERVAL
        } else {
            interval <= MAX_LEGACY_REPEAT_INTERVAL
                && self.previous_interval.is_some_and(|initial_delay| {
                    initial_delay >= MIN_LEGACY_INITIAL_DELAY
                        && initial_delay >= interval.saturating_mul(2)
                })
        };

        if self.legacy_repeat && !repeated {
            // A long gap after a recognized held stream is a new physical
            // press, not the initial delay of a continuation of that stream.
            self.previous_interval = None;
        } else {
            self.previous_interval = Some(interval);
        }
        self.last_press = Some(now);
        self.legacy_repeat = repeated;
        repeated
    }

    fn reset(&mut self) {
        *self = Self::default();
    }
}

fn motion_repeat_dispatches(app: &App, input: &InputEvent, repeated: bool) -> usize {
    if !repeated || app.has_input_overlay() {
        return 1;
    }
    let InputEvent::Key(key) = input else {
        return 1;
    };
    let sequence = KeySequence::from(*key);
    let binding = match app
        .keymap()
        .lookup_in(app.mode, app.key_binding_scope(), &sequence)
    {
        Lookup::Exact(binding) | Lookup::ExactAndPrefix { exact: binding, .. } => binding,
        Lookup::NoMatch | Lookup::Prefix(_) => return 1,
    };
    if binding.availability.is_implemented()
        && matches!(
            binding.target,
            BindingTarget::Editor(command)
                if command.category() == CommandCategory::Movement
                    && !matches!(
                        command,
                        runyte::command::EditorCommand::MoveFileStart
                            | runyte::command::EditorCommand::MoveFileEnd
                    )
        )
    {
        app.config.editor.motion_repeat_multiplier.max(1)
    } else {
        1
    }
}

struct TerminalGuard {
    mouse_enabled: bool,
    keyboard_enhancement: bool,
}

#[cfg(unix)]
fn keyboard_enhancement_flags() -> KeyboardEnhancementFlags {
    keyboard_enhancement_flags_for(cfg!(target_os = "macos"))
}

#[cfg(unix)]
fn keyboard_enhancement_flags_for(legacy_repeat_cadence: bool) -> KeyboardEnhancementFlags {
    if legacy_repeat_cadence {
        // macOS terminals have not been reliable sources of explicit repeat
        // and release events. Disambiguation alone is enough to encode Ctrl
        // chords, including the terminal pane keys, without opting plain
        // typing into that event stream. Unsupported terminals ignore the
        // request and retain Crossterm's legacy control-byte decoding.
        return KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
            | KeyboardEnhancementFlags::REPORT_ALTERNATE_KEYS;
    }
    // Reporting every key encodes a shifted printable key from its unshifted
    // codepoint. The alternate codepoint is what lets Crossterm recover the
    // character produced by the active layout, such as `:` from Shift-`;`.
    KeyboardEnhancementFlags::REPORT_EVENT_TYPES
        | KeyboardEnhancementFlags::REPORT_ALTERNATE_KEYS
        | KeyboardEnhancementFlags::REPORT_ALL_KEYS_AS_ESCAPE_CODES
}

impl TerminalGuard {
    fn enter(mouse_enabled: bool) -> Result<Self> {
        enable_raw_mode().context("failed to enable terminal raw mode")?;
        let mut output = stdout();
        if let Err(error) = output.execute(EnterAlternateScreen) {
            let _ = disable_raw_mode();
            return Err(error).context("failed to enter alternate screen");
        }
        // macOS uses the disambiguation-only profile above: it makes Ctrl
        // chords deterministic without requesting the unreliable repeat and
        // release stream. The cadence detector remains its repeat fallback.
        #[cfg(unix)]
        let keyboard_enhancement = {
            let flags = keyboard_enhancement_flags();
            if let Err(error) = output.execute(PushKeyboardEnhancementFlags(flags)) {
                let _ = output.execute(LeaveAlternateScreen);
                let _ = disable_raw_mode();
                return Err(error).context("failed to enable enhanced keyboard reporting");
            }
            true
        };
        #[cfg(not(unix))]
        let keyboard_enhancement = false;
        if let Err(error) = output.execute(EnableBracketedPaste) {
            #[cfg(unix)]
            if keyboard_enhancement {
                let _ = output.execute(PopKeyboardEnhancementFlags);
            }
            let _ = output.execute(LeaveAlternateScreen);
            let _ = disable_raw_mode();
            return Err(error).context("failed to enable bracketed paste");
        }
        if mouse_enabled && let Err(error) = output.execute(EnableMouseCapture) {
            let _ = output.execute(DisableBracketedPaste);
            #[cfg(unix)]
            if keyboard_enhancement {
                let _ = output.execute(PopKeyboardEnhancementFlags);
            }
            let _ = output.execute(LeaveAlternateScreen);
            let _ = disable_raw_mode();
            return Err(error).context("failed to enable mouse capture");
        }
        Ok(Self {
            mouse_enabled,
            keyboard_enhancement,
        })
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let mut output = stdout();
        if self.mouse_enabled {
            let _ = output.execute(DisableMouseCapture);
        }
        let _ = output.execute(DisableBracketedPaste);
        #[cfg(unix)]
        if self.keyboard_enhancement {
            let _ = output.execute(PopKeyboardEnhancementFlags);
        }
        let _ = output.execute(Show);
        let _ = output.execute(LeaveAlternateScreen);
        let _ = disable_raw_mode();
    }
}

/// Tells the person, once, that the diagnostic log stopped working.
///
/// A destination that becomes unwritable after startup is seen only by the
/// background writer. Editing and serving continue either way, so this is a
/// notification rather than an error: stderr belongs to the terminal the TUI
/// is drawing on, and `:service-health` already carries the standing state.
fn report_logging_failure(app: &mut App) {
    if let Some(failure) = diagnostic_log::unreported_failure() {
        app.push_notification(NotificationDraft::new(
            NotificationSeverity::Warning,
            "Logging",
            "Diagnostic log stopped recording",
            format!("{failure} · editing continues without a durable log"),
        ));
    }
}

/// Records a background service ending exactly once.
///
/// The editor keeps working without it, so nothing else reports the loss; a
/// detached host would otherwise show only the missing behaviour.
fn note_ended_service(
    reported: &mut std::collections::HashSet<&'static str>,
    service: &'static str,
) {
    if reported.insert(service) {
        log_warn!("service", "background service ended"; "service" => service);
    }
}

/// Says plainly that an already-running host kept its own logging.
///
/// Verbosity and destination are properties of host startup. Accepting `-v`
/// or `--log` silently here would present an attachment as if it had
/// reconfigured the host that owns the workspace.
fn report_retained_logging(verbosity: u8, log: Option<&Path>) {
    if verbosity == 0 && log.is_none() {
        return;
    }
    eprintln!(
        "runyte: the running session kept its own log level and destination; \
restart it with --session-restart to change them"
    );
}

fn report_retained_host_logging(arguments: &LaunchArguments) {
    report_retained_logging(arguments.verbosity, arguments.log.as_deref());
}

/// The same report for callers that hold an endpoint rather than a mode.
#[cfg(unix)]
async fn report_retained_logging_if_running(
    endpoint: &LocalEndpoint,
    verbosity: u8,
    log: Option<&Path>,
) {
    if (verbosity == 0 && log.is_none()) || connect_control(endpoint).await.is_err() {
        return;
    }
    report_retained_logging(verbosity, log);
}

/// Installs the diagnostic logger this process will own, if any.
///
/// Returns the failure text when the default destination could not be used.
/// A failed default degrades logging rather than preventing editing or
/// preventing a host from serving; a failed explicit `--log` is a startup
/// error, because silently choosing another destination would make the
/// requested capture misleading.
fn initialize_logging(
    arguments: &LaunchArguments,
    role: LogRole,
    state_root: &Path,
    project_root: &Path,
) -> Result<Option<String>> {
    let level = LogLevel::from_verbosity(arguments.verbosity);
    let path = arguments
        .log
        .clone()
        .unwrap_or_else(|| diagnostic_log::default_path(state_root, role, std::process::id()));
    // Every launch leaves a standalone log behind, so the directory is swept
    // before this one is opened. Live owners are never touched.
    if role == LogRole::Standalone && arguments.log.is_none() {
        diagnostic_log::prune_standalone_logs(
            state_root,
            std::process::id(),
            diagnostic_log::RETAINED_STANDALONE_LOGS,
        );
    }
    let workspace = workspace_id(project_root);
    let abbreviated = workspace
        .get(..ABBREVIATED_LOG_WORKSPACE_ID)
        .unwrap_or(&workspace)
        .to_owned();
    let settings = diagnostic_log::Settings::new(level, role).with_workspace(Some(abbreviated));
    let sink = if arguments.log.is_some() {
        diagnostic_log::Sink::exclusive_file(path.clone())
    } else {
        diagnostic_log::Sink::file(path.clone())
    };
    match diagnostic_log::Logger::start(settings, sink) {
        Ok(logger) => {
            diagnostic_log::install(logger);
            diagnostic_log::install_panic_hook();
            Ok(None)
        }
        Err(failure) => {
            anyhow::ensure!(arguments.log.is_none(), "{failure}");
            diagnostic_log::note_unavailable(role, Some(path), failure.clone());
            // Reported here, so the periodic check does not repeat it as a
            // second notification once an `App` exists.
            diagnostic_log::note_failure_reported();
            eprintln!("runyte: {failure}");
            Ok(Some(failure))
        }
    }
}

/// How much of the workspace ID each record carries. The same abbreviation
/// session listings show, so a record can be matched to a listed session
/// without pasting a 32-character hash onto every line. The startup record
/// carries the complete ID.
const ABBREVIATED_LOG_WORKSPACE_ID: usize = 8;

fn print_help() {
    println!(
        "\
runyte — a fast modal terminal editor

USAGE:
    runyte [OPTIONS] [+LINE[:COLUMN] FILE]... [-- FILE...]

OPTIONS:
    -c, --config PATH    Use a specific YAML config
    -i, --init DIRECTORY Initialize and open DIRECTORY as a workspace
    -v, --verbose        Raise the diagnostic log level; repeat for more
        --log PATH       Write the diagnostic log to PATH instead
    -h, --help           Print help
    -V, --version        Print version

MODES:
    A workspace is one project directory plus its live editor state. Standalone
    mode keeps that state in the TUI process. Persistent mode keeps it alive
    between TUIs and is currently available only on Unix.

        --standalone     Use standalone mode, overriding configuration
    -a, --persistent [WORKSPACE]
                         Attach to the selected or current session, starting it
                         if needed

PERSISTENT SESSIONS:
    A persistent session is the durable local process and retained editor state
    associated with one workspace. Listing also works from standalone mode; attaching,
    starting, and stopping inside the editor need
    workspace.mode: persistent inside the editor.

    WORKSPACE selects a session by ID, unambiguous ID prefix, persistent name,
    or directory, so a session is reachable from anywhere. An existing directory
    names that exact workspace; its configured state directory is created when
    necessary. Omitting WORKSPACE uses the workspace found from the current
    directory, or initializes that directory when none is found. Use --init to
    initialize and open an exact directory in standalone mode.

        --serve          Keep a persistent session alive in the foreground
        --wait FILE...   Edit files through persistent mode and wait for
                         explicit completion
    -l, --session-list   List running and recently visited sessions
        --session-start [WORKSPACE]
                         Start the selected or current session
    -s, --session-stop [WORKSPACE]
                         Stop the selected or current session
        --session-stop-all
                         Stop every running session in the current namespace
        --all-namespaces With session-list or session-stop-all, include every
                         validated live session owned by the current user
        --session-clear-all
                         Clear every stopped session from the recent list
        --session-restart [WORKSPACE]
                         Restart the selected or current session
        --session-rename WORKSPACE NAME
                         Rename a persistent session
    -f, --force          With stop/stop-all/restart, discard protected buffers,
                         waiters, and live terminal children

DIAGNOSTICS:
    Runyte keeps a small local log of warnings, errors, and, when asked, more
    detailed lifecycle events. The process that owns editor state owns the
    file: a standalone editor writes .runyte/standalone-<pid>.log, a persistent
    session writes .runyte/host.log. At most 4 MiB is kept in the active file
    and 4 MiB in one previous file beside it. A standalone launch keeps the
    four newest logs left by exited standalone processes and removes older
    active and previous files without touching a live owner's log.

    The default level records warnings and errors. Each -v raises it through
    info, debug, and trace, and stops at trace. --log PATH selects another
    destination; a path that cannot be written is a startup error, while an
    unwritable default only degrades logging. On Unix, a path already owned by
    another running Runyte process is refused.

    In persistent mode these are properties of session startup. --serve,
    --session-start, --session-restart, and the launch that starts a missing
    session pass them on; attaching to a running session leaves its logging
    alone and says so. Inside the editor, :log-open shows the log of the
    process that owns the workspace and :service-health names its owner,
    level, and path.

    Records never contain document text, selections, clipboard or terminal
    contents, environment values, or language-server message bodies. They do
    contain local paths and process metadata, so review a log before sharing
    it.

TARGETS:
    (no target)          Open the Runyte about page
    DIRECTORY            Open DIRECTORY in the explorer
    +LINE[:COLUMN] FILE  Open FILE and place its caret at a one-based position
    -- FILE...           Treat every remaining argument as a literal path

    Naming a target always runs standalone, so its relative path and caret
    position keep their ordinary meaning: workspace.mode: persistent changes
    only a bare runyte, and --persistent reads its argument as a workspace
    rather than a file. Use --wait to open files through a persistent session.

:quit-here moves the shell to the editor's directory on exit; it requires the
runyte() shell function documented in README.md.

Inside the editor press Space+? for the complete key reference."
    );
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::{Path, PathBuf},
        time::{Duration, Instant, SystemTime, UNIX_EPOCH},
    };

    #[cfg(unix)]
    use crossterm::event::{
        KeyboardEnhancementFlags, PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
    };
    use crossterm::{
        Command,
        event::{
            DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
            KeyEventKind,
        },
    };

    #[cfg(unix)]
    use super::keyboard_enhancement_flags_for;
    #[cfg(unix)]
    use super::{
        AttachedClient, AttachedWorkspaceActivity, HostResponse, PointerBatcher, WaitStatus,
        WaitToken, atomic_write_cwd_file_with, dispatch_host_key_or_text,
        recover_switched_attachment, send_active_response, workspace_response_publishes_frame,
    };
    use super::{
        KeyRepeatDetector, initialize_attached_directory, is_passive_pointer, is_redraw_only_event,
        motion_repeat_dispatches, observe_key_or_text_hint, rejected_text_input,
        resolve_cwd_file_path, resolve_requested_project_root, starts_on_about,
        uses_automatic_persistent_mode, write_cwd_file,
    };
    use runyte::launch::LaunchArguments;
    use runyte::{
        app::App,
        config::{Config, WorkspaceMode},
        input::{InputEvent, KeyCode, KeyStroke, Modifiers, PointerEvent, PointerEventKind},
        key_hints::KeyHintState,
        selection::Selection,
        text::Transaction,
        tui::input::convert_event,
        workspace::WorkspaceHost,
    };

    #[cfg(unix)]
    #[test]
    fn read_only_wait_responses_do_not_publish_editor_frames() {
        let token: WaitToken = serde_json::from_str("1").unwrap();
        let response = HostResponse::WaitState {
            token,
            status: WaitStatus::Pending {
                buffers: Vec::new(),
                remaining: Vec::new(),
            },
            interactive_attached: true,
        };

        assert!(!workspace_response_publishes_frame(&response));
    }

    #[cfg(unix)]
    #[test]
    fn established_attachment_activity_records_arrival_and_every_kind_of_departure() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        static RECORDS: AtomicUsize = AtomicUsize::new(0);
        fn record(_: &Path) -> anyhow::Result<()> {
            RECORDS.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }

        RECORDS.store(0, Ordering::SeqCst);
        {
            let _activity = AttachedWorkspaceActivity::begin_with(Path::new("/workspace"), record);
            assert_eq!(RECORDS.load(Ordering::SeqCst), 1, "arrival is recorded");
            // Dropping the guard models every return as well as cancellation
            // of the attachment future by a signal.
        }
        assert_eq!(RECORDS.load(Ordering::SeqCst), 2, "departure is recorded");
    }

    #[cfg(unix)]
    #[test]
    fn failed_switched_attachment_restores_its_source() {
        use runyte::workspace::transport::LocalEndpoint;

        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "runyte-switch-recovery-{}-{nanos}",
            std::process::id()
        ));
        let runtime = root.join("runtime");
        let source_root = root.join("source");
        let destination_root = root.join("destination");
        fs::create_dir_all(&source_root).unwrap();
        fs::create_dir_all(&destination_root).unwrap();
        let source = LocalEndpoint::discover_with_runtime(
            &source_root.join(".runyte"),
            &source_root,
            Some(&runtime),
        )
        .unwrap();
        let mut current = LocalEndpoint::discover_with_runtime(
            &destination_root.join(".runyte"),
            &destination_root,
            Some(&runtime),
        )
        .unwrap();
        let mut previous = Some(source);
        let mut notice = None;

        let outcome = recover_switched_attachment::<()>(
            Err(anyhow::anyhow!("destination handshake failed")),
            &mut current,
            &mut previous,
            &mut notice,
        )
        .unwrap();

        assert!(outcome.is_none());
        assert_eq!(current.project_root(), source_root);
        assert!(previous.is_none());
        assert_eq!(notice.as_deref(), Some("destination handshake failed"));

        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn a_client_that_is_only_behind_keeps_its_attachment() {
        use runyte::app::FrameGeometry;
        use runyte::workspace::transport::HostResponse;

        let mut host = WorkspaceHost::new(App::new(Config::default(), None).unwrap());
        let hints = KeyHintState::default();
        let frame = HostResponse::Frame {
            frame: Box::new(
                host.prepare_frame_with_hints(FrameGeometry::default(), Some(&hints))
                    .into(),
            ),
        };
        let client = |responses| AttachedClient {
            id: 1,
            geometry: FrameGeometry::default(),
            responses,
            wait_tokens: Vec::new(),
            last_frame: None,
        };

        // Visual responses have one replaceable slot, so a repaint burst
        // retains only the latest complete/damage state and never detaches.
        let (responses, _receiver) = runyte::workspace::transport::response_channel();
        let fill = responses.clone();
        let mut active = Some(client(responses));
        send_active_response(&mut active, frame.clone());
        assert!(active.is_some());
        send_active_response(&mut active, frame.clone());
        assert!(
            active.is_some(),
            "replacing a pending frame detached a live client"
        );

        // A control message carries state the client cannot reconstruct, so a
        // channel still full at this depth is reported rather than silently
        // dropping it.
        for index in 0..64 {
            fill.try_send(HostResponse::Error {
                message: index.to_string(),
            })
            .unwrap();
        }
        send_active_response(
            &mut active,
            HostResponse::Error {
                message: "boom".to_owned(),
            },
        );
        assert!(active.is_none(), "a lost control message went unreported");

        // A closed connection is the one case that really means gone.
        let (responses, receiver) = runyte::workspace::transport::response_channel();
        drop(receiver);
        let mut active = Some(client(responses));
        send_active_response(&mut active, frame);
        assert!(active.is_none(), "a closed connection stayed attached");
    }

    #[test]
    fn paste_and_mouse_lifecycle_commands_are_available_and_inverse() {
        let mut enable = String::new();
        let mut disable = String::new();
        EnableBracketedPaste.write_ansi(&mut enable).unwrap();
        DisableBracketedPaste.write_ansi(&mut disable).unwrap();

        assert_eq!(enable, "\u{1b}[?2004h");
        assert_eq!(disable, "\u{1b}[?2004l");

        enable.clear();
        disable.clear();
        EnableMouseCapture.write_ansi(&mut enable).unwrap();
        DisableMouseCapture.write_ansi(&mut disable).unwrap();
        assert!(!enable.is_empty());
        assert!(!disable.is_empty());
        assert_ne!(enable, disable);
    }

    #[cfg(unix)]
    #[test]
    fn keyboard_reporting_profiles_keep_macos_control_keys_unambiguous_without_event_types() {
        let macos = keyboard_enhancement_flags_for(true);
        assert_eq!(
            macos,
            KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
                | KeyboardEnhancementFlags::REPORT_ALTERNATE_KEYS
        );
        assert!(!macos.contains(KeyboardEnhancementFlags::REPORT_EVENT_TYPES));
        assert!(!macos.contains(KeyboardEnhancementFlags::REPORT_ALL_KEYS_AS_ESCAPE_CODES));
        let mut macos_enable = String::new();
        PushKeyboardEnhancementFlags(macos)
            .write_ansi(&mut macos_enable)
            .unwrap();
        assert_eq!(macos_enable, "\u{1b}[>5u");

        let full = keyboard_enhancement_flags_for(false);
        let mut disable = String::new();
        assert_eq!(
            full,
            KeyboardEnhancementFlags::REPORT_EVENT_TYPES
                | KeyboardEnhancementFlags::REPORT_ALTERNATE_KEYS
                | KeyboardEnhancementFlags::REPORT_ALL_KEYS_AS_ESCAPE_CODES
        );
        let mut full_enable = String::new();
        PushKeyboardEnhancementFlags(full)
            .write_ansi(&mut full_enable)
            .unwrap();
        PopKeyboardEnhancementFlags
            .write_ansi(&mut disable)
            .unwrap();
        assert_eq!(full_enable, "\u{1b}[>14u");
        assert!(!disable.is_empty());
        assert_ne!(full_enable, disable);
        assert_ne!(macos_enable, disable);
    }

    #[test]
    fn held_single_key_motions_use_the_configured_multiplier_only_on_repeats() {
        let mut config = Config::default();
        config.editor.motion_repeat_multiplier = 4;
        let app = App::new(config, None).unwrap();
        let left = InputEvent::Key(KeyStroke::plain(KeyCode::Left));
        let modal_left = InputEvent::Key(KeyStroke::char('h'));
        let insert = InputEvent::Key(KeyStroke::char('i'));

        assert_eq!(motion_repeat_dispatches(&app, &left, true), 4);
        assert_eq!(motion_repeat_dispatches(&app, &modal_left, true), 4);
        assert_eq!(motion_repeat_dispatches(&app, &left, false), 1);
        assert_eq!(motion_repeat_dispatches(&app, &insert, true), 1);
    }

    #[test]
    fn held_file_boundary_keys_are_not_replayed() {
        let mut config = Config::default();
        config.editor.motion_repeat_multiplier = 10;
        let app = App::new(config, None).unwrap();

        for key in [KeyStroke::char('G'), KeyStroke::char('g')] {
            let input = InputEvent::Key(key);
            assert_eq!(motion_repeat_dispatches(&app, &input, true), 1);
        }
    }

    #[cfg(unix)]
    #[test]
    fn attached_host_input_builds_the_same_key_hint_state() {
        let mut config = Config::default();
        config.editor.motion_repeat_multiplier = 3;
        let mut app = App::new(config, None).unwrap();
        app.buffers[0].apply(&Transaction::insert(0, "abcdef"));
        app.panes.get_mut(&0).unwrap().selection = Selection::point(5);
        let mut host = WorkspaceHost::new(app);
        let mut hints = KeyHintState::default();
        dispatch_host_key_or_text(
            &mut host,
            &mut hints,
            InputEvent::Key(KeyStroke::char('g')),
            false,
        );
        let frame =
            host.prepare_frame_with_hints(runyte::app::FrameGeometry::default(), Some(&hints));
        assert!(
            frame
                .overlays
                .iter()
                .any(|overlay| overlay.kind == runyte::snapshot::OverlayKind::KeyHints)
        );
        dispatch_host_key_or_text(
            &mut host,
            &mut hints,
            InputEvent::Key(KeyStroke::plain(KeyCode::Left)),
            true,
        );
        assert!(host.active().head() < 4, "repeat input was not accelerated");
    }

    #[test]
    fn macro_owned_input_clears_hints_before_frontend_dispatch() {
        let mut app = App::new(Config::default(), None).unwrap();
        for character in [' ', 'm', 'm', 'l', ' ', 'm', 'm', ' ', 'm', 'r'] {
            app.handle_key(KeyStroke::char(character)).unwrap();
        }
        assert!(app.macro_replay_pending());

        let mut hints = KeyHintState::default();
        hints.push(KeyStroke::char('g'));
        assert!(hints.is_pending());

        assert_eq!(
            observe_key_or_text_hint(&app, &mut hints, &InputEvent::Key(KeyStroke::char('g')),),
            runyte::key_hints::HintEventResult::Forward
        );
        assert!(!hints.is_pending());
    }

    #[cfg(unix)]
    #[test]
    fn attached_host_treats_replacement_space_as_character_input() {
        let mut app = App::new(Config::default(), None).unwrap();
        app.buffers[0].apply(&Transaction::insert(0, "ab"));
        let mut host = WorkspaceHost::new(app);
        let mut hints = KeyHintState::default();

        for key in ['r', ' '] {
            dispatch_host_key_or_text(
                &mut host,
                &mut hints,
                InputEvent::Key(KeyStroke::char(key)),
                false,
            );
        }
        assert_eq!(host.buffers[0].text().to_string(), " b");
        assert!(!hints.is_visible());

        dispatch_host_key_or_text(
            &mut host,
            &mut hints,
            InputEvent::Key(KeyStroke::char(' ')),
            false,
        );
        assert_eq!(hints.display_pending(), "Space");
        assert!(hints.is_visible());
    }

    #[test]
    fn legacy_press_cadence_identifies_a_held_motion_without_accelerating_taps() {
        let key = InputEvent::Key(KeyStroke::char('j'));
        let start = Instant::now();
        let mut detector = KeyRepeatDetector::default();

        assert!(!detector.observe(Some(KeyEventKind::Press), Some(&key), start));
        assert!(!detector.observe(
            Some(KeyEventKind::Press),
            Some(&key),
            start + Duration::from_millis(500),
        ));
        assert!(detector.observe(
            Some(KeyEventKind::Press),
            Some(&key),
            start + Duration::from_millis(533),
        ));
        assert!(detector.observe(
            Some(KeyEventKind::Press),
            Some(&key),
            start + Duration::from_millis(566),
        ));

        let mut config = Config::default();
        config.editor.motion_repeat_multiplier = 4;
        let app = App::new(config, None).unwrap();
        assert_eq!(motion_repeat_dispatches(&app, &key, true), 4);

        assert!(!detector.observe(Some(KeyEventKind::Release), None, start));
        assert!(!detector.observe(
            Some(KeyEventKind::Press),
            Some(&key),
            start + Duration::from_millis(600),
        ));

        let mut taps = KeyRepeatDetector::default();
        for elapsed in [0, 50, 100, 150] {
            assert!(!taps.observe(
                Some(KeyEventKind::Press),
                Some(&key),
                start + Duration::from_millis(elapsed),
            ));
        }
    }

    #[test]
    fn enhanced_repeat_events_remain_authoritative() {
        let key = InputEvent::Key(KeyStroke::plain(KeyCode::Down));
        let start = Instant::now();
        let mut detector = KeyRepeatDetector::default();

        assert!(!detector.observe(Some(KeyEventKind::Press), Some(&key), start));
        assert!(detector.observe(
            Some(KeyEventKind::Repeat),
            Some(&key),
            start + Duration::from_millis(500),
        ));
        assert!(!detector.observe(
            Some(KeyEventKind::Release),
            None,
            start + Duration::from_millis(533),
        ));
    }

    #[test]
    fn non_key_input_resets_legacy_repeat_history() {
        let key = InputEvent::Key(KeyStroke::char('j'));
        let text = InputEvent::Text("paste".to_owned());
        let start = Instant::now();
        let mut detector = KeyRepeatDetector::default();

        assert!(!detector.observe(Some(KeyEventKind::Press), Some(&key), start));
        assert!(!detector.observe(None, Some(&text), start + Duration::from_millis(200)));
        assert!(!detector.observe(
            Some(KeyEventKind::Press),
            Some(&key),
            start + Duration::from_millis(210),
        ));
        assert!(!detector.observe(
            Some(KeyEventKind::Press),
            Some(&key),
            start + Duration::from_millis(220),
        ));
    }

    #[test]
    fn text_input_limit_is_shared_before_standalone_or_attached_dispatch() {
        let exact = InputEvent::Text("x".repeat(runyte::input::MAX_TEXT_INPUT_BYTES));
        let oversized = InputEvent::Text("x".repeat(runyte::input::MAX_TEXT_INPUT_BYTES + 1));

        assert_eq!(rejected_text_input(&exact), None);
        assert_eq!(
            rejected_text_input(&oversized).as_deref(),
            Some("text input exceeds the 1048576 byte limit")
        );
        assert_eq!(
            rejected_text_input(&InputEvent::Key(KeyStroke::char('x'))),
            None
        );
    }

    #[test]
    fn attached_none_input_release_resets_repeat_cadence() {
        let key = InputEvent::Key(KeyStroke::plain(KeyCode::Down));
        let start = Instant::now();
        let mut detector = KeyRepeatDetector::default();
        assert!(!detector.observe(Some(KeyEventKind::Press), Some(&key), start));
        assert!(detector.observe(
            Some(KeyEventKind::Repeat),
            Some(&key),
            start + Duration::from_millis(500),
        ));

        let release = crossterm::event::Event::Key(crossterm::event::KeyEvent::new_with_kind(
            crossterm::event::KeyCode::Down,
            crossterm::event::KeyModifiers::NONE,
            KeyEventKind::Release,
        ));
        let converted = convert_event(release).unwrap();
        assert!(converted.is_none());
        // The attached loop must still reach the detector before continuing.
        assert!(!detector.observe(
            Some(KeyEventKind::Release),
            converted.as_ref(),
            start + Duration::from_millis(533),
        ));
        assert!(!detector.observe(
            Some(KeyEventKind::Press),
            Some(&key),
            start + Duration::from_millis(566),
        ));
    }

    #[test]
    fn passive_pointer_motion_is_not_an_editor_or_redraw_event() {
        assert!(is_passive_pointer(&InputEvent::Pointer(PointerEvent {
            kind: PointerEventKind::Moved,
            column: 12,
            row: 7,
            modifiers: Modifiers::NONE,
        })));
        assert!(!is_passive_pointer(&InputEvent::Pointer(PointerEvent {
            kind: PointerEventKind::ScrollDown,
            column: 12,
            row: 7,
            modifiers: Modifiers::NONE,
        })));
    }

    #[cfg(unix)]
    #[test]
    fn attached_pointer_batcher_coalesces_only_identical_wheel_input() {
        use runyte::app::FrameGeometry;

        let mut host = WorkspaceHost::new(App::new(Config::default(), None).unwrap());
        let first = host.prepare_frame(FrameGeometry::default()).id;
        let second = host.prepare_frame(FrameGeometry::default()).id;
        let down = PointerEvent {
            kind: PointerEventKind::ScrollDown,
            column: 12,
            row: 7,
            modifiers: Modifiers::NONE,
        };
        let up = PointerEvent {
            kind: PointerEventKind::ScrollUp,
            ..down
        };
        let mut batcher = PointerBatcher::default();

        assert_eq!(batcher.push_wheel(down, first), None);
        assert_eq!(batcher.push_wheel(down, second), None);
        assert_eq!(batcher.push_wheel(up, second).unwrap().repetitions, 2);
        let pending = batcher.take().unwrap();
        assert_eq!(pending.event, up);
        assert_eq!(pending.frame, second);
        assert_eq!(pending.repetitions, 1);
    }

    #[test]
    fn a_resize_carries_no_input_but_still_redraws() {
        let resize = crossterm::event::Event::Resize(120, 40);
        // The event produces no editor input, so only the redraw predicate
        // keeps the loop from leaving the previous shape on screen.
        assert!(
            convert_event(resize.clone())
                .expect("resize converts")
                .is_none()
        );
        assert!(is_redraw_only_event(&resize));

        for quiet in [
            crossterm::event::Event::FocusGained,
            crossterm::event::Event::FocusLost,
        ] {
            assert!(
                convert_event(quiet.clone())
                    .expect("focus converts")
                    .is_none()
            );
            assert!(!is_redraw_only_event(&quiet));
        }

        assert!(!is_redraw_only_event(&crossterm::event::Event::Key(
            crossterm::event::KeyEvent::new(
                crossterm::event::KeyCode::Char('a'),
                crossterm::event::KeyModifiers::NONE,
            )
        )));
    }

    #[test]
    fn cwd_file_option_preserves_its_path() {
        let arguments = LaunchArguments::parse_from([
            "--cwd-file".into(),
            "/tmp/runyte cwd".into(),
            "notes.txt".into(),
        ])
        .unwrap();

        assert_eq!(arguments.cwd_file, Some(PathBuf::from("/tmp/runyte cwd")));
        assert_eq!(arguments.targets[0].path, PathBuf::from("notes.txt"));

        let arguments = LaunchArguments::parse_from(["--cwd-file=/tmp/runyte cwd".into()]).unwrap();
        assert_eq!(arguments.cwd_file, Some(PathBuf::from("/tmp/runyte cwd")));
    }

    #[test]
    fn project_root_option_carries_a_resolved_workspace() {
        let arguments = LaunchArguments::parse_from([
            "--serve".into(),
            "--project-root".into(),
            "/tmp/runyte project".into(),
        ])
        .unwrap();

        assert_eq!(
            arguments.project_root,
            Some(PathBuf::from("/tmp/runyte project"))
        );

        assert!(LaunchArguments::parse_from(["--project-root".into()]).is_err());
        assert!(LaunchArguments::parse_from(["--project-root".into(), "".into()]).is_err());
    }

    #[test]
    fn an_attachment_directory_is_initialized_as_the_exact_workspace_root() {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "runyte-attach-selector-{}-{nanos}",
            std::process::id()
        ));
        let project = root.join("project");
        let nested = project.join("src").join("deep");
        let plain = root.join("plain");
        fs::create_dir_all(&nested).unwrap();
        fs::create_dir_all(project.join(".git")).unwrap();
        fs::write(project.join(".git").join("HEAD"), "ref: refs/heads/main\n").unwrap();
        fs::create_dir_all(&plain).unwrap();
        let state = PathBuf::from(".runyte");
        let expected_project = project.canonicalize().unwrap();
        let expected_nested = nested.canonicalize().unwrap();
        let expected_plain = plain.canonicalize().unwrap();

        assert_eq!(
            initialize_attached_directory(&project, &project, &state, &[]).unwrap(),
            expected_project
        );
        assert!(project.join(".runyte").is_dir());

        // Unlike ordinary discovery, an explicitly named nested directory is
        // the workspace root even when a Git root exists above it.
        assert_eq!(
            initialize_attached_directory(&nested, &nested, &state, &[]).unwrap(),
            expected_nested
        );
        assert!(nested.join(".runyte").is_dir());

        assert_eq!(
            initialize_attached_directory(&plain, &plain, &state, &[]).unwrap(),
            expected_plain
        );
        assert!(plain.join(".runyte").is_dir());

        // An ID or name that matched nothing is not a directory either.
        let error = initialize_attached_directory(
            Path::new("no-such-session"),
            Path::new("no-such-session"),
            &state,
            &[],
        )
        .unwrap_err();
        assert!(error.to_string().contains("--session-list"), "{error}");

        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_requested_project_root_must_contain_the_launch_directory() {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "runyte-requested-root-{}-{nanos}",
            std::process::id()
        ));
        let project = root.join("project");
        let nested = project.join("nested");
        let outside = root.join("outside");
        fs::create_dir_all(&nested).unwrap();
        fs::create_dir_all(&outside).unwrap();
        let project = project.canonicalize().unwrap();

        assert_eq!(
            resolve_requested_project_root(&nested.canonicalize().unwrap(), &project).unwrap(),
            project
        );
        // A root that does not contain the launch directory would give this
        // process a workspace identity belonging to another project.
        let error = resolve_requested_project_root(&outside.canonicalize().unwrap(), &project)
            .unwrap_err()
            .to_string();
        assert!(error.contains("outside project root"), "{error}");
        // A file, and a path that is not there at all, are refused rather than
        // silently becoming the launch directory.
        let file = project.join("note.txt");
        fs::write(&file, "base\n").unwrap();
        assert!(resolve_requested_project_root(&project, &file).is_err());
        assert!(resolve_requested_project_root(&project, &root.join("missing")).is_err());

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn relative_cwd_file_keeps_the_invoking_shells_identity_after_directory_changes() {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "runyte-cwd-replacement-{}-{nanos}",
            std::process::id()
        ));
        let invoking_directory = root.join("shell");
        let destination = root.join("destination");
        fs::create_dir_all(invoking_directory.join("state")).unwrap();
        fs::create_dir_all(destination.join("state")).unwrap();

        let first = resolve_cwd_file_path(&invoking_directory, PathBuf::from("state/cwd"));
        let forwarded = resolve_cwd_file_path(&destination, first.clone());
        let selected_directory = destination.join("selected");
        write_cwd_file(&forwarded, &selected_directory).unwrap();

        assert_eq!(first, invoking_directory.join("state/cwd"));
        assert_eq!(forwarded, first);
        assert_ne!(forwarded, destination.join("state/cwd"));
        assert!(invoking_directory.join("state/cwd").is_file());
        assert!(!destination.join("state/cwd").exists());

        let mut expected = selected_directory.as_os_str().as_encoded_bytes().to_vec();
        #[cfg(unix)]
        expected.push(0);
        assert_eq!(fs::read(&first).unwrap(), expected);

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn absolute_cwd_file_is_forwarded_unchanged() {
        let path = std::env::temp_dir().join("shell/state/runyte-cwd");

        assert_eq!(
            resolve_cwd_file_path(std::env::temp_dir().as_path(), path.clone()),
            path
        );
    }

    #[test]
    fn targetless_standalone_launches_open_about_but_paths_keep_their_meaning() {
        let bare = LaunchArguments::parse_from([]).unwrap();
        let explicit_standalone = LaunchArguments::parse_from(["--standalone".into()]).unwrap();
        let directory = LaunchArguments::parse_from([".".into()]).unwrap();
        let file = LaunchArguments::parse_from(["file.txt".into()]).unwrap();
        let server = LaunchArguments::parse_from(["--serve".into()]).unwrap();

        assert!(starts_on_about(&bare));
        assert!(starts_on_about(&explicit_standalone));
        assert!(!starts_on_about(&directory));
        assert!(!starts_on_about(&file));
        assert!(!starts_on_about(&server));
    }

    #[test]
    fn persistent_default_only_changes_bare_implicit_launches() {
        let bare = LaunchArguments::parse_from([]).unwrap();
        let file = LaunchArguments::parse_from(["note.txt".into()]).unwrap();
        let directory = LaunchArguments::parse_from([".".into()]).unwrap();
        let positioned = LaunchArguments::parse_from(["+4:2".into(), "note.txt".into()]).unwrap();
        let explicit_standalone = LaunchArguments::parse_from(["--standalone".into()]).unwrap();

        assert!(uses_automatic_persistent_mode(
            &bare,
            WorkspaceMode::Persistent
        ));
        assert!(!uses_automatic_persistent_mode(
            &file,
            WorkspaceMode::Persistent
        ));
        assert!(!uses_automatic_persistent_mode(
            &directory,
            WorkspaceMode::Persistent
        ));
        assert!(!uses_automatic_persistent_mode(
            &positioned,
            WorkspaceMode::Persistent
        ));
        assert!(!uses_automatic_persistent_mode(
            &explicit_standalone,
            WorkspaceMode::Persistent
        ));
        assert!(!uses_automatic_persistent_mode(
            &bare,
            WorkspaceMode::Standalone
        ));
    }

    #[test]
    fn cwd_file_preserves_the_encoded_path_and_platform_terminator() {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root =
            std::env::temp_dir().join(format!("runyte-cwd-file-{}-{nanos}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let output = root.join("cwd");
        let directory = root.join("directory with spaces");

        fs::write(&output, b"stale").unwrap();
        write_cwd_file(&output, &directory).unwrap();
        let mut expected = directory.as_os_str().as_encoded_bytes().to_vec();
        #[cfg(unix)]
        expected.push(0);
        assert_eq!(fs::read(&output).unwrap(), expected);
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(&output).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(fs::read_dir(&root).unwrap().count(), 1);

        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn cwd_file_retry_preserves_colliding_temporary_file() {
        let root = std::env::temp_dir().join(format!(
            "runyte-cwd-collision-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        let collision = root.join(format!(".runyte-cwd-{}-41.tmp", std::process::id()));
        fs::write(&collision, b"sentinel").unwrap();
        let target = root.join("cwd");
        let mut sequences = [41, 42].into_iter();

        atomic_write_cwd_file_with(&target, b"replacement", || sequences.next().unwrap()).unwrap();

        assert_eq!(fs::read(&target).unwrap(), b"replacement");
        assert_eq!(fs::read(&collision).unwrap(), b"sentinel");
        assert!(
            !root
                .join(format!(".runyte-cwd-{}-42.tmp", std::process::id()))
                .exists()
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn cwd_file_supports_a_near_name_max_target() {
        let root = std::env::temp_dir().join(format!(
            "runyte-cwd-long-name-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        let target = root.join("x".repeat(250));

        write_cwd_file(&target, Path::new("/tmp/destination")).unwrap();

        assert!(target.is_file());
        fs::remove_dir_all(root).unwrap();
    }
}
