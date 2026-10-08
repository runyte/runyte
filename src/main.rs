// SPDX-License-Identifier: MPL-2.0

#[cfg(any(unix, windows))]
mod host_requests;
#[cfg(windows)]
#[cfg_attr(not(test), allow(dead_code))]
#[path = "tui/windows_frontend.rs"]
mod windows_frontend;
#[cfg(windows)]
mod windows_host;
#[cfg(unix)]
use host_requests::{bounded_destination_label, handle_workspace_request, is_workspace_request};

#[cfg(all(test, windows))]
#[path = "tui/windows_console_acceptance.rs"]
mod windows_console_acceptance;

#[cfg(all(test, windows))]
#[path = "tui/windows_frontend_acceptance.rs"]
mod windows_frontend_acceptance;

#[cfg(all(test, windows))]
#[path = "tui/windows_git_acceptance.rs"]
mod windows_git_acceptance;

use std::{
    io::{self, Write, stdout},
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

#[cfg(any(not(windows), debug_assertions, test))]
use std::fs;

#[cfg(unix)]
use std::thread;

use anyhow::{Context, Result};
#[cfg(not(windows))]
use crossterm::event::EventStream;
#[cfg(unix)]
use crossterm::event::{
    KeyboardEnhancementFlags, PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
};
use crossterm::{
    ExecutableCommand, QueueableCommand,
    cursor::{Hide, MoveTo, Show},
    event::{
        DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
        Event as CrosstermEvent, KeyEventKind,
    },
    style::{Attribute, Print, SetAttribute},
    terminal::{
        Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode,
        enable_raw_mode,
    },
};
#[cfg(windows)]
use futures_util::FutureExt;
#[cfg(not(windows))]
use futures_util::StreamExt;
use ratatui::{Terminal, backend::CrosstermBackend};
use runyte::{
    app::App,
    command::{
        CommandCategory, CommandExecutionContext, CommandInvocation, CommandInvocationError,
        EditorCommand,
    },
    config::{self, Config, RunMode},
    external_open, file_monitor, file_picker,
    git::{GitCliProvider, GitService, GitServiceEvent},
    git_monitor,
    input::{InputEvent, KeyStroke, PointerEventKind},
    key_hints::{HintEventResult, KeyHintState},
    keymap::{BindingTarget, KeySequence, Lookup},
    launch::{LaunchArguments, LaunchMode},
    log::{self as diagnostic_log, Level as LogLevel, Role as LogRole},
    log_debug, log_error, log_info, log_trace, log_warn,
    lsp::{self, LspCommand, LspHandle},
    notification::{NotificationDraft, NotificationSeverity},
    project_root,
    startup::{StartupPhase, StartupTrace},
    syntax::{self, SyntaxEvents},
    terminal::{self, TerminalEvents},
    tui::input::convert_event,
    ui, word_index,
    workspace::{HostCommand, HostEvent, HostInputOutcome, WorkspaceHost, workspace_id},
};
#[cfg(unix)]
use runyte::{app::PersistentExitRequest, input::PointerEvent, launch::LaunchTarget};

const STATUS_ANIMATION_INTERVAL: Duration = Duration::from_millis(80);
/// How often work that arrives faster than anyone can read it is allowed to
/// present a frame.
///
/// A child writing continuously, and a live-content scan advancing in small
/// row slices, both produce far more states than a reader can follow. Drawing
/// each one turns a busy terminal or a long scan into a flicker, so they mark
/// a frame pending and this interval decides when it is drawn.
const FRAME_INTERVAL: Duration = Duration::from_millis(16);
const MAINTENANCE_INTERVAL: Duration = Duration::from_secs(1);
/// How often the finder re-reads terminals that are still producing output.
///
/// A running child changes the corpus faster than the list can be read, and a
/// finder whose rows move on every chunk a child writes is unusable however
/// cheap the rebuild is. The finder trades freshness for a list that holds
/// still: this bounds how often its terminal rows can change. Long enough
/// that a reader who has found their row keeps it while they read it, since
/// stale terminal output costs nothing next to a list that moves under them.
const FINDER_TERMINAL_REFRESH_INTERVAL: Duration = Duration::from_secs(5);
#[cfg(unix)]
const WAIT_LIFECYCLE_RECOVERY_BUDGET: Duration = Duration::from_millis(500);
/// How long a shutting-down host waits for its connections to finish writing.
/// Longer than the transport's own write stall budget, so a peer that is
/// merely slow is flushed and only one that has stopped reading is cut off.
#[cfg(unix)]
const SHUTDOWN_FLUSH_BUDGET: Duration = Duration::from_secs(3);

/// Decides at the publication boundary whether a requested frame is whole.
///
/// A terminal-content refill temporarily removes the rows it is about to read
/// back. Any event can request a frame while that bounded scan is in flight,
/// so filtering only the scan and frame-tick branches is not enough: the final
/// draw or publish site must defer every request until the refill completes.
fn frame_publication_ready(
    requested: bool,
    finder_refilling: bool,
    frame_pending: &mut bool,
) -> bool {
    if !requested {
        return false;
    }
    if finder_refilling {
        *frame_pending = true;
        return false;
    }
    true
}

/// Scanner and ranker progress can arrive much faster than a terminal can
/// present it. Input takes a different select branch and still draws
/// immediately; only these background intermediate states use frame pacing.
fn pace_file_picker_event(event: &file_picker::FilePickerEvent) -> bool {
    matches!(
        event,
        file_picker::FilePickerEvent::Files { .. }
            | file_picker::FilePickerEvent::Content { .. }
            | file_picker::FilePickerEvent::Ranked { .. }
    )
}

#[cfg(unix)]
use runyte::protocol::{MAX_POINTER_REPETITIONS, WaitStatus, WaitToken, validate_welcome};
#[cfg(unix)]
use runyte::workspace::lifecycle::{
    HostStartup, UnavailableStartupExecutable, connect_control, force_restart_host,
    force_shutdown_host, resolve_registered_host, resolve_registered_host_from_directory,
    resolve_workspace_endpoint, restart_host, shutdown_host, start_detached_host,
    terminate_incompatible_host,
};
#[cfg(unix)]
use runyte::workspace::transport::{
    BufferedLocalClient, ClientRequest, FeatureGroup, HostResponse, IncompatibleHost, LocalClient,
    LocalEndpoint, LocalServer, ServerEvent, decode_path, encode_path,
    registered_hosts_all_namespaces,
};
#[cfg(unix)]
use runyte::workspace::{
    WorkspaceRow, WorkspaceService, abbreviated_id_width, clear_stopped_sessions,
    ensure_recent_workspace, known_workspaces, known_workspaces_all_namespaces,
    known_workspaces_for_navigation, record_recent_workspace, record_workspace_activity,
    rename_known_workspace, resolve_known_workspace, resolve_known_workspace_from_directory,
};
#[cfg(windows)]
use runyte::workspace::{
    normalize_session_name,
    windows_catalog::{HistoryTarget, WorkspaceRow, abbreviated_id_width},
    windows_control::{ControlSnapshot, StopAllReport, UserSelector},
    windows_endpoint::EndpointLocation,
    windows_location::{CapturedRoots, DiscoveryInputs, DiscoveryScope, ResolvedLayout},
    windows_parent_identity::ForegroundParentSupervisor,
    windows_startup::{self, HostStartup as NativeHostStartup},
};

#[cfg(feature = "native")]
mod native_frontend;

fn main() -> Result<()> {
    #[cfg(feature = "native")]
    if std::env::args_os()
        .nth(1)
        .is_some_and(|arg| arg == "--native-pdf-helper")
    {
        return native_frontend::pdf::helper_main(std::env::args_os().skip(2));
    }
    // `runed` is `runyte --editor`, so `runed mcp` opens a file named mcp
    // just as `runyte --editor mcp` does; editor mode has no MCP.
    #[cfg(any(unix, windows))]
    if std::env::args_os().nth(1).is_some_and(|arg| arg == "mcp")
        && !runyte::launch::started_as_runed()
    {
        return runyte::mcp::run(std::env::args_os().skip(2));
    }
    #[cfg(feature = "native")]
    if std::env::args_os()
        .skip(1)
        .take_while(|arg| arg != "--")
        .any(|arg| arg == "--window")
    {
        let arguments = LaunchArguments::parse()?;
        if arguments.window && !arguments.help && !arguments.version {
            anyhow::ensure!(
                !cfg!(windows),
                "the native-window experiment currently supports Linux and macOS"
            );
            anyhow::ensure!(
                matches!(
                    arguments.mode,
                    LaunchMode::Standalone | LaunchMode::Persistent
                ),
                "--window supports ide, editor, and mux modes"
            );
            let (config, _) = Config::load(arguments.config.as_deref())?;
            return native_frontend::launch(cli_main, config.editor.font_size);
        }
    }
    cli_main()
}

fn cli_main() -> Result<()> {
    let mut startup = StartupTrace::new();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("failed to start the async runtime")?;
    #[cfg(windows)]
    let mut native_termination = runtime.block_on(async { TerminationSignals::new() })?;
    #[cfg(windows)]
    let result = runtime.block_on(run(&mut startup, &mut native_termination));
    #[cfg(not(windows))]
    let result = runtime.block_on(run(&mut startup));
    #[cfg(windows)]
    let result = reconcile_pending_console_event(
        result,
        runtime.block_on(native_termination.pending_event()),
    );
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
    #[cfg(windows)]
    let result = reconcile_pending_console_event(result, native_termination.recv().now_or_never());
    #[cfg(windows)]
    drop(native_termination);
    #[cfg(windows)]
    if result.as_ref().err().is_some_and(console_closed) {
        // A closed console may no longer accept Rust's top-level Result
        // reporter. Cleanup and logging already ran; avoid writing to it.
        std::process::exit(1);
    }
    #[cfg(unix)]
    if let Err(error) = &result
        && let Some(signal) = error.downcast_ref::<TerminatedBySignal>()
    {
        std::process::exit(128 + signal.0);
    }
    #[cfg(unix)]
    if result
        .as_ref()
        .is_err_and(|error| error.downcast_ref::<WaitTerminalLost>().is_some())
    {
        // The terminal is already gone, so returning this error would make
        // Rust's top-level Result reporter write it to a dead stderr. That
        // write panics and changes the intended failure into exit status 101.
        std::process::exit(1);
    }
    result
}

#[cfg(not(windows))]
#[derive(Debug)]
struct TerminatedBySignal(i32);

#[cfg(not(windows))]
impl std::fmt::Display for TerminatedBySignal {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "terminated by signal {}", self.0)
    }
}

#[cfg(not(windows))]
impl std::error::Error for TerminatedBySignal {}

#[cfg(windows)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ConsoleEvent {
    CtrlC,
    CtrlBreak,
    Close,
}

#[cfg(windows)]
#[derive(Debug)]
struct TerminatedByConsole(ConsoleEvent);

#[cfg(windows)]
impl std::fmt::Display for TerminatedByConsole {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self.0 {
            ConsoleEvent::CtrlC => "terminated by Ctrl+C",
            ConsoleEvent::CtrlBreak => "terminated by Ctrl+Break",
            ConsoleEvent::Close => "console closed",
        })
    }
}

#[cfg(windows)]
impl std::error::Error for TerminatedByConsole {}

#[cfg(windows)]
fn console_closed(error: &anyhow::Error) -> bool {
    error
        .downcast_ref::<TerminatedByConsole>()
        .is_some_and(|event| event.0 == ConsoleEvent::Close)
}

#[cfg(windows)]
fn prefer_pending_console_close(result: Result<()>, pending: Option<ConsoleEvent>) -> Result<()> {
    if pending != Some(ConsoleEvent::Close) || result.as_ref().err().is_some_and(console_closed) {
        return result;
    }
    let close = terminated(ConsoleEvent::Close);
    match result {
        Ok(()) => Err(close),
        Err(previous) => Err(close.context(format!("previous shutdown result: {previous:#}"))),
    }
}

#[cfg(windows)]
fn reconcile_pending_console_event(
    result: Result<()>,
    pending: Option<ConsoleEvent>,
) -> Result<()> {
    if pending == Some(ConsoleEvent::Close) {
        return prefer_pending_console_close(result, pending);
    }
    match (result, pending) {
        (Ok(()), Some(event)) => Err(terminated(event)),
        (result, _) => result,
    }
}

#[cfg(windows)]
fn finish_standalone_native(
    outcome: Result<()>,
    context: Result<()>,
    catalog: Result<()>,
    plugins: Result<()>,
    event: Option<ConsoleEvent>,
) -> Result<()> {
    let mut result = event.map_or(outcome, |event| Err(terminated(event)));
    for (label, cleanup) in [
        ("context service shutdown failed", context),
        ("native catalog shutdown failed", catalog),
        ("plugin shutdown failed", plugins),
    ] {
        if let Err(error) = cleanup {
            result = match result {
                Ok(()) => Err(error.context(label)),
                Err(primary) => Err(primary.context(format!("{label}: {error}"))),
            };
        }
    }
    result
}

#[cfg(unix)]
#[derive(Debug)]
struct WaitTerminalLost;

#[cfg(unix)]
impl std::fmt::Display for WaitTerminalLost {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("wait request lost its terminal before completion")
    }
}

#[cfg(unix)]
impl std::error::Error for WaitTerminalLost {}

#[cfg(unix)]
struct TerminationSignals {
    reader: tokio::io::unix::AsyncFd<std::os::unix::net::UnixStream>,
    writer: std::os::unix::net::UnixStream,
}

/// Restores a terminal synchronously when startup is interrupted before the
/// async event loop can receive the signal wake.
#[cfg(unix)]
struct StartupSignalExit;

#[cfg(unix)]
impl StartupSignalExit {
    fn arm() -> Result<Self> {
        let mut attributes = std::mem::MaybeUninit::<libc::termios>::uninit();
        // SAFETY: `attributes` points to writable storage and stdout is the
        // terminal this standalone process is about to place in raw mode.
        if unsafe { libc::tcgetattr(libc::STDOUT_FILENO, attributes.as_mut_ptr()) } == -1 {
            return Err(std::io::Error::last_os_error())
                .context("failed to preserve terminal state for startup");
        }
        // SAFETY: successful `tcgetattr` initialized `attributes`. No signal
        // reads this slot until the release-store below publishes it.
        unsafe {
            std::ptr::addr_of_mut!(STARTUP_TERMINAL_STATE)
                .write(std::mem::MaybeUninit::new(attributes.assume_init()));
        }
        STARTUP_TERMINAL_ACTIVE.store(true, std::sync::atomic::Ordering::Release);
        Ok(Self)
    }
}

#[cfg(unix)]
impl Drop for StartupSignalExit {
    fn drop(&mut self) {
        STARTUP_TERMINAL_ACTIVE.store(false, std::sync::atomic::Ordering::Release);
    }
}

#[cfg(not(unix))]
struct StartupSignalExit;

#[cfg(not(unix))]
impl StartupSignalExit {
    fn arm() -> Result<Self> {
        Ok(Self)
    }
}

#[cfg(unix)]
impl TerminationSignals {
    fn new() -> Result<Self> {
        use std::os::fd::AsRawFd;

        let (reader, writer) = std::os::unix::net::UnixStream::pair()?;
        reader.set_nonblocking(true)?;
        writer.set_nonblocking(true)?;
        let reader = tokio::io::unix::AsyncFd::new(reader)?;
        install_termination_handlers()?;
        TERMINATION_WRITE_FD.store(writer.as_raw_fd(), std::sync::atomic::Ordering::Release);
        Ok(Self { reader, writer })
    }

    async fn recv(&mut self) -> i32 {
        loop {
            if let Some(signal) = self.received() {
                return signal;
            }
            let mut ready = self
                .reader
                .readable()
                .await
                .expect("termination signal descriptor remains readable");
            ready.clear_ready();
        }
    }

    fn received(&mut self) -> Option<i32> {
        use std::io::Read;

        let mut wake = [0_u8; 64];
        match self.reader.get_mut().read(&mut wake) {
            Ok(_) | Err(_) => {}
        }
        let signal = RECEIVED_TERMINATION.swap(0, std::sync::atomic::Ordering::AcqRel);
        (signal != 0).then_some(signal)
    }
}

#[cfg(unix)]
impl Drop for TerminationSignals {
    fn drop(&mut self) {
        use std::os::fd::AsRawFd;

        if TERMINATION_WRITE_FD
            .compare_exchange(
                self.writer.as_raw_fd(),
                -1,
                std::sync::atomic::Ordering::AcqRel,
                std::sync::atomic::Ordering::Relaxed,
            )
            .is_ok()
        {
            // A handler that entered before the descriptor was unpublished
            // may still be writing it. Wait before `writer` closes so the fd
            // cannot be reused underneath that async-signal-safe write.
            while TERMINATION_HANDLERS_ACTIVE.load(std::sync::atomic::Ordering::Acquire) != 0 {
                std::hint::spin_loop();
            }
        }
    }
}

/// Reports when the terminal behind stdin is no longer reachable.
///
/// Crossterm 0.29 keeps its Unix `EventStream` alive after a zero-byte terminal
/// read. A hung-up PTY is then continuously readable, so its helper thread can
/// spin without ever producing an event for the async caller. Watching only
/// exceptional poll states avoids competing with Crossterm for input and also
/// covers a `--wait` client that is still queued behind another interactive
/// TUI and has not created an event stream yet.
#[cfg(unix)]
struct TerminalLoss {
    cancel: Option<std::os::unix::net::UnixStream>,
    event: Option<tokio::sync::oneshot::Receiver<std::io::Result<()>>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

#[cfg(unix)]
impl TerminalLoss {
    fn new() -> Result<Self> {
        use std::os::fd::AsRawFd;

        let Some(terminal) = wait_client_terminal()? else {
            return Ok(Self {
                cancel: None,
                event: None,
                thread: None,
            });
        };
        let (cancel, cancel_reader) = std::os::unix::net::UnixStream::pair()
            .context("cannot create terminal-loss cancellation socket")?;
        let (sender, event) = tokio::sync::oneshot::channel();
        let thread = std::thread::Builder::new()
            .name("runyte-terminal-loss".to_owned())
            .spawn(move || {
                if let Some(result) =
                    wait_for_terminal_loss(terminal.as_raw_fd(), cancel_reader.as_raw_fd())
                {
                    let _ = sender.send(result);
                }
            })
            .context("cannot start terminal-loss watcher")?;
        Ok(Self {
            cancel: Some(cancel),
            event: Some(event),
            thread: Some(thread),
        })
    }

    async fn recv(&mut self) -> Result<()> {
        let Some(event) = self.event.as_mut() else {
            return std::future::pending().await;
        };
        match event.await {
            Ok(result) => result.context("failed while watching the terminal"),
            Err(_) => anyhow::bail!("terminal-loss watcher stopped unexpectedly"),
        }
    }
}

/// Opens the same terminal source Crossterm uses and gives the watcher its own
/// close-on-exec descriptor. A queued noninteractive wait can legitimately
/// have neither a TTY stdin nor `/dev/tty`; it remains usable through the
/// already-attached TUI, and a later attempted takeover fails through the
/// existing terminal-entry path.
#[cfg(unix)]
fn wait_client_terminal() -> Result<Option<std::os::fd::OwnedFd>> {
    use std::os::fd::{FromRawFd, IntoRawFd};

    // SAFETY: `isatty` only inspects the process's standard input descriptor.
    if unsafe { libc::isatty(libc::STDIN_FILENO) } == 1 {
        // SAFETY: `F_DUPFD_CLOEXEC` duplicates a live descriptor and returns a
        // fresh owned descriptor on success.
        let descriptor = unsafe { libc::fcntl(libc::STDIN_FILENO, libc::F_DUPFD_CLOEXEC, 0) };
        if descriptor == -1 {
            return Err(std::io::Error::last_os_error())
                .context("cannot duplicate the wait client's terminal");
        }
        // SAFETY: successful duplication transferred one fresh descriptor.
        return Ok(Some(unsafe {
            std::os::fd::OwnedFd::from_raw_fd(descriptor)
        }));
    }

    match fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/tty")
    {
        Ok(terminal) => {
            // SAFETY: `into_raw_fd` transfers the file's one owned descriptor.
            Ok(Some(unsafe {
                std::os::fd::OwnedFd::from_raw_fd(terminal.into_raw_fd())
            }))
        }
        Err(error)
            if matches!(
                error.raw_os_error(),
                Some(libc::ENOENT | libc::ENXIO | libc::ENODEV | libc::ENOTTY)
            ) =>
        {
            Ok(None)
        }
        Err(error) => Err(error).context("cannot open the wait client's terminal"),
    }
}

/// Blocks without consuming input until `terminal` reports loss, or until
/// `cancel` becomes readable. `None` is the ordinary cancellation path.
#[cfg(all(unix, not(target_os = "macos")))]
fn wait_for_terminal_loss(
    terminal: std::os::fd::RawFd,
    cancel: std::os::fd::RawFd,
) -> Option<std::io::Result<()>> {
    let mut descriptors = [
        libc::pollfd {
            fd: terminal,
            events: 0,
            revents: 0,
        },
        libc::pollfd {
            fd: cancel,
            events: libc::POLLIN,
            revents: 0,
        },
    ];
    loop {
        // SAFETY: both entries contain live descriptors for this call, and the
        // array is writable storage for the two returned `revents` fields.
        let result = unsafe { libc::poll(descriptors.as_mut_ptr(), descriptors.len() as _, -1) };
        if result == -1 {
            let error = std::io::Error::last_os_error();
            if error.kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            return Some(Err(error));
        }
        if descriptors[1].revents != 0 {
            return None;
        }
        if descriptors[0].revents & (libc::POLLHUP | libc::POLLERR | libc::POLLNVAL) != 0 {
            return Some(Ok(()));
        }
    }
}

/// Darwin's `poll` adapter does not register a descriptor whose requested
/// event mask is zero. Asking it for read or hangup readiness is not safe
/// either: the adapter uses a one-shot read knote, so ordinary unread input can
/// consume the observation before a later PTY close. A native kqueue watcher
/// can ignore ordinary readability, clear that notification without consuming
/// input, and wait for the distinct EOF transition.
#[cfg(target_os = "macos")]
fn wait_for_terminal_loss(
    terminal: std::os::fd::RawFd,
    cancel: std::os::fd::RawFd,
) -> Option<std::io::Result<()>> {
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};

    // SAFETY: `kqueue` has no preconditions and returns a fresh descriptor.
    let descriptor = unsafe { libc::kqueue() };
    if descriptor == -1 {
        return Some(Err(std::io::Error::last_os_error()));
    }
    // SAFETY: a successful `kqueue` call returns a fresh owned descriptor.
    let queue = unsafe { OwnedFd::from_raw_fd(descriptor) };
    let changes = [
        libc::kevent {
            ident: terminal as libc::uintptr_t,
            filter: libc::EVFILT_READ,
            flags: libc::EV_ADD | libc::EV_ENABLE | libc::EV_CLEAR,
            fflags: 0,
            data: 0,
            udata: std::ptr::null_mut(),
        },
        libc::kevent {
            ident: cancel as libc::uintptr_t,
            filter: libc::EVFILT_READ,
            flags: libc::EV_ADD | libc::EV_ENABLE,
            fflags: 0,
            data: 0,
            udata: std::ptr::null_mut(),
        },
    ];
    // SAFETY: `queue` is live and `changes` contains two complete read-filter
    // registrations. This call supplies no event output storage.
    if unsafe {
        libc::kevent(
            queue.as_raw_fd(),
            changes.as_ptr(),
            changes.len() as _,
            std::ptr::null_mut(),
            0,
            std::ptr::null(),
        )
    } == -1
    {
        return Some(Err(std::io::Error::last_os_error()));
    }

    let mut events = [
        libc::kevent {
            ident: 0,
            filter: 0,
            flags: 0,
            fflags: 0,
            data: 0,
            udata: std::ptr::null_mut(),
        },
        libc::kevent {
            ident: 0,
            filter: 0,
            flags: 0,
            fflags: 0,
            data: 0,
            udata: std::ptr::null_mut(),
        },
    ];
    loop {
        // SAFETY: `queue` remains live and `events` has writable storage for
        // both returned events. A null timeout blocks until one is ready.
        let count = unsafe {
            libc::kevent(
                queue.as_raw_fd(),
                std::ptr::null(),
                0,
                events.as_mut_ptr(),
                events.len() as _,
                std::ptr::null(),
            )
        };
        if count == -1 {
            let error = std::io::Error::last_os_error();
            if error.kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            return Some(Err(error));
        }
        let ready = &events[..count as usize];
        if let Some(event) = ready.iter().find(|event| event.flags & libc::EV_ERROR != 0) {
            let error = i32::try_from(event.data)
                .ok()
                .filter(|code| *code > 0)
                .map(std::io::Error::from_raw_os_error)
                .unwrap_or_else(|| std::io::Error::from_raw_os_error(libc::EIO));
            return Some(Err(error));
        }
        // Preserve the poll implementation's cancellation priority when the
        // cancellation socket and terminal EOF become ready together.
        if ready.iter().any(|event| {
            event.ident == cancel as libc::uintptr_t && event.filter == libc::EVFILT_READ
        }) {
            return None;
        }
        if ready.iter().any(|event| {
            event.ident == terminal as libc::uintptr_t
                && event.filter == libc::EVFILT_READ
                && event.flags & libc::EV_EOF != 0
        }) {
            return Some(Ok(()));
        }
    }
}

#[cfg(unix)]
impl Drop for TerminalLoss {
    fn drop(&mut self) {
        if let Some(cancel) = self.cancel.as_mut() {
            let _ = cancel.write_all(&[0]);
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[cfg(unix)]
async fn terminal_loss_error(termination: &mut TerminationSignals) -> anyhow::Error {
    // A controlling-PTY close reports descriptor hangup and SIGHUP together.
    // Give the signal handler one bounded scheduling window so its established
    // 128+signal process status wins that race. Descriptor loss without a
    // signal still exits promptly through the generic error below.
    tokio::select! {
        biased;
        signal = termination.recv() => terminated(signal),
        _ = tokio::time::sleep(Duration::from_millis(50)) => {
            WaitTerminalLost.into()
        }
    }
}

#[cfg(unix)]
static RECEIVED_TERMINATION: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);

#[cfg(unix)]
static TERMINATION_WRITE_FD: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(-1);

#[cfg(unix)]
static TERMINATION_HANDLERS_ACTIVE: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);

#[cfg(unix)]
static STARTUP_TERMINAL_ACTIVE: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

#[cfg(unix)]
static mut STARTUP_TERMINAL_STATE: std::mem::MaybeUninit<libc::termios> =
    std::mem::MaybeUninit::uninit();

#[cfg(unix)]
unsafe fn exit_from_interrupted_startup(signal: libc::c_int) -> ! {
    const RESTORE_PRESENTATION: &[u8] = b"\x1b[?1000l\x1b[?1002l\x1b[?1003l\
        \x1b[?1015l\x1b[?1006l\x1b[?2004l\x1b[<u\x1b[?25h\x1b[?1049l";
    // SAFETY: the active flag publishes a fully initialized termios value.
    // `tcsetattr`, `write`, and `_exit` are async-signal-safe on POSIX. This
    // path never returns to the interrupted synchronous file or parser work.
    unsafe {
        let attributes = std::ptr::addr_of!(STARTUP_TERMINAL_STATE).cast::<libc::termios>();
        let _ = libc::tcsetattr(libc::STDOUT_FILENO, libc::TCSANOW, attributes);
        let _ = libc::write(
            libc::STDOUT_FILENO,
            RESTORE_PRESENTATION.as_ptr().cast(),
            RESTORE_PRESENTATION.len(),
        );
        libc::_exit(128 + signal);
    }
}

#[cfg(unix)]
extern "C" fn record_termination(signal: libc::c_int) {
    if STARTUP_TERMINAL_ACTIVE.swap(false, std::sync::atomic::Ordering::AcqRel) {
        // SAFETY: `StartupSignalExit::arm` published the saved state before it
        // set the active flag, and this branch does not return.
        unsafe { exit_from_interrupted_startup(signal) };
    }
    RECEIVED_TERMINATION.store(signal, std::sync::atomic::Ordering::Release);
    TERMINATION_HANDLERS_ACTIVE.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
    let fd = TERMINATION_WRITE_FD.load(std::sync::atomic::Ordering::Acquire);
    if fd >= 0 {
        let wake = signal as u8;
        // SAFETY: `wake` is live for this one-byte write and `fd` names the
        // non-blocking signal socket installed by `TerminationSignals::new`.
        // `write` is async-signal-safe; a full socket can drop this byte
        // because the atomic signal value remains pending.
        let _ = unsafe { libc::write(fd, (&wake as *const u8).cast(), 1) };
    }
    TERMINATION_HANDLERS_ACTIVE.fetch_sub(1, std::sync::atomic::Ordering::Release);
}

#[cfg(unix)]
fn install_termination_handlers() -> Result<()> {
    static INSTALLED: std::sync::OnceLock<std::result::Result<(), i32>> =
        std::sync::OnceLock::new();
    let result = INSTALLED.get_or_init(|| {
        // SAFETY: a zeroed sigaction is a valid starting point; the handler
        // only performs lock-free atomics and an async-signal-safe write.
        let mut action: libc::sigaction = unsafe { std::mem::zeroed() };
        action.sa_sigaction = record_termination as *const () as usize;
        action.sa_flags = 0;
        // SAFETY: `sa_mask` is owned writable storage in `action`.
        if unsafe { libc::sigemptyset(&mut action.sa_mask) } == -1 {
            return Err(std::io::Error::last_os_error().raw_os_error().unwrap_or(0));
        }
        for signal in [libc::SIGINT, libc::SIGHUP, libc::SIGTERM] {
            // SAFETY: `action` remains initialized for each call, and a null
            // final argument discards the old disposition.
            if unsafe { libc::sigaction(signal, &action, std::ptr::null_mut()) } == -1 {
                return Err(std::io::Error::last_os_error().raw_os_error().unwrap_or(0));
            }
        }
        Ok(())
    });
    result.map_err(|code| std::io::Error::from_raw_os_error(code).into())
}

#[cfg(not(windows))]
fn terminated(signal: i32) -> anyhow::Error {
    TerminatedBySignal(signal).into()
}

#[cfg(windows)]
fn terminated(event: ConsoleEvent) -> anyhow::Error {
    TerminatedByConsole(event).into()
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
    pidfd: Option<tokio::io::unix::AsyncFd<std::os::fd::OwnedFd>>,
    #[cfg(target_os = "macos")]
    process_queue: Option<tokio::io::unix::AsyncFd<std::os::fd::OwnedFd>>,
}

#[cfg(unix)]
impl HostSupervisor {
    fn for_launch(arguments: &LaunchArguments) -> Result<Option<Self>> {
        if arguments.mode == LaunchMode::Wait {
            if let Some(value) = std::env::var_os("RUNYTE_TEST_WAIT_PARENT_PID") {
                let pid = value
                    .to_string_lossy()
                    .parse::<libc::pid_t>()
                    .context("RUNYTE_TEST_WAIT_PARENT_PID must be a positive process ID")?;
                anyhow::ensure!(pid > 0, "RUNYTE_TEST_WAIT_PARENT_PID must be positive");
                return Self::new(HostSupervisorKind::TestProcess, pid).map(Some);
            }
            // SAFETY: `getppid` has no preconditions and only reads process
            // metadata maintained by the kernel.
            return Self::new(HostSupervisorKind::Parent, unsafe { libc::getppid() }).map(Some);
        }
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
        let pidfd = open_pidfd(pid)?
            .map(tokio::io::unix::AsyncFd::new)
            .transpose()
            .context("cannot register host supervisor process descriptor")?;
        #[cfg(target_os = "macos")]
        let process_queue = open_process_queue(pid)?
            .map(|queue| {
                tokio::io::unix::AsyncFd::with_interest(queue, tokio::io::Interest::READABLE)
            })
            .transpose()
            .context("cannot register host supervisor process queue")?;
        Ok(Self {
            kind,
            pid,
            #[cfg(target_os = "linux")]
            pidfd,
            #[cfg(target_os = "macos")]
            process_queue,
        })
    }

    fn exited(&self) -> Result<bool> {
        #[cfg(target_os = "linux")]
        if let Some(pidfd) = self.pidfd.as_ref()
            && pidfd_has_exited(pidfd.get_ref())
        {
            return Ok(true);
        }
        #[cfg(target_os = "macos")]
        if let Some(process_queue) = self.process_queue.as_ref()
            && process_queue_has_exited(process_queue.get_ref(), self.pid)?
        {
            return Ok(true);
        }
        Ok(match self.kind {
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
        })
    }

    fn pid(&self) -> libc::pid_t {
        self.pid
    }

    async fn recv(&self) -> Result<()> {
        #[cfg(target_os = "linux")]
        if let Some(pidfd) = self.pidfd.as_ref() {
            loop {
                let mut ready = pidfd.readable().await?;
                if pidfd_has_exited(pidfd.get_ref()) {
                    return Ok(());
                }
                ready.clear_ready();
            }
        }
        #[cfg(target_os = "macos")]
        if let Some(process_queue) = self.process_queue.as_ref() {
            loop {
                let mut ready = process_queue.readable().await?;
                if process_queue_has_exited(process_queue.get_ref(), self.pid)? {
                    return Ok(());
                }
                ready.clear_ready();
            }
        }
        // Stable kernel observation can be unavailable under restricted
        // kernels. This fallback is deliberately coarse and is used only then;
        // ordinary Linux/macOS waits block on the descriptor above.
        loop {
            tokio::time::sleep(Duration::from_millis(500)).await;
            if self.exited()? {
                return Ok(());
            }
        }
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
fn process_queue_has_exited(
    process_queue: &std::os::fd::OwnedFd,
    pid: libc::pid_t,
) -> Result<bool> {
    use std::os::fd::AsRawFd;

    let timeout = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    process_queue_has_exited_with(pid, |event| {
        // SAFETY: the kqueue descriptor is live, the output has capacity for
        // one event, and the zero timeout performs a non-blocking observation.
        let count = unsafe {
            libc::kevent(
                process_queue.as_raw_fd(),
                std::ptr::null(),
                0,
                event.as_mut_ptr(),
                1,
                &timeout,
            )
        };
        if count == -1 {
            Err(std::io::Error::last_os_error())
        } else {
            Ok(count)
        }
    })
}

#[cfg(target_os = "macos")]
fn process_queue_has_exited_with(
    pid: libc::pid_t,
    mut observe: impl FnMut(&mut std::mem::MaybeUninit<libc::kevent>) -> std::io::Result<libc::c_int>,
) -> Result<bool> {
    // A signal can interrupt this nonblocking read without consuming the
    // NOTE_EXIT event. Retry the same retained queue rather than consulting a
    // PID that could later name another process.
    let (count, event) = loop {
        let mut event = std::mem::MaybeUninit::<libc::kevent>::uninit();
        match observe(&mut event) {
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error).context("cannot read host supervisor process queue"),
            Ok(count) => break (count, event),
        }
    };
    if count == 0 {
        return Ok(false);
    }
    // SAFETY: `kevent` returned one event into the initialized output slot.
    let event = unsafe { event.assume_init() };
    if event.flags & libc::EV_ERROR != 0 {
        let error = i32::try_from(event.data)
            .ok()
            .filter(|code| *code > 0)
            .map(std::io::Error::from_raw_os_error)
            .unwrap_or_else(|| std::io::Error::from_raw_os_error(libc::EIO));
        return Err(error).context("host supervisor process queue reported an error");
    }
    Ok(event.ident == pid as libc::uintptr_t
        && event.filter == libc::EVFILT_PROC
        && event.fflags & libc::NOTE_EXIT != 0)
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

#[cfg(windows)]
struct TerminationSignals {
    ctrl_c: tokio::signal::windows::CtrlC,
    ctrl_break: tokio::signal::windows::CtrlBreak,
    close: tokio::signal::windows::CtrlClose,
}

#[cfg(windows)]
impl TerminationSignals {
    fn new() -> Result<Self> {
        // One owner is installed before launch parsing and stays alive through
        // startup and cleanup. Tokio's close handler preserves its callback
        // thread for the operating system's limited cleanup window.
        Ok(Self {
            ctrl_c: tokio::signal::windows::ctrl_c()?,
            ctrl_break: tokio::signal::windows::ctrl_break()?,
            close: tokio::signal::windows::ctrl_close()?,
        })
    }

    async fn recv(&mut self) -> ConsoleEvent {
        tokio::select! {
            biased;
            _ = self.close.recv() => ConsoleEvent::Close,
            _ = self.ctrl_break.recv() => ConsoleEvent::CtrlBreak,
            _ = self.ctrl_c.recv() => ConsoleEvent::CtrlC,
        }
    }

    async fn pending_event(&mut self) -> Option<ConsoleEvent> {
        tokio::select! {
            biased;
            event = self.recv() => Some(event),
            _ = std::future::ready(()) => None,
        }
    }
}

#[cfg(all(not(unix), not(windows)))]
struct TerminationSignals;

#[cfg(all(not(unix), not(windows)))]
impl TerminationSignals {
    fn new() -> Result<Self> {
        Ok(Self)
    }

    async fn recv(&mut self) -> i32 {
        std::future::pending().await
    }
}

async fn run(
    startup: &mut StartupTrace,
    #[cfg(windows)] native_termination: &mut TerminationSignals,
) -> Result<()> {
    let mut arguments = LaunchArguments::parse()?;
    #[cfg(not(feature = "native"))]
    anyhow::ensure!(
        !arguments.window,
        "--window requires cargo build --features native"
    );
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

    // Capture a foreground host's natural parent before configuration or App
    // startup. Detached hosts have a disposable inheritance parent and never
    // turn that process into a supervisor.
    #[cfg(windows)]
    if arguments.mode == LaunchMode::Serve {
        let supervisor = if arguments.detached_host {
            None
        } else {
            Some(ForegroundParentSupervisor::capture()?)
        };
        return windows_host::run(arguments, startup, native_termination, supervisor).await;
    }

    #[cfg(windows)]
    let mut native_preloaded_config = None;
    #[cfg(windows)]
    if arguments.mode == LaunchMode::Standalone
        && !arguments.mode_explicit
        && arguments.targets.is_empty()
        && arguments.init.is_none()
    {
        let loaded = Config::load(arguments.config.as_deref())?;
        if uses_automatic_persistent_mode(&arguments, loaded.0.mode()) {
            arguments.mode = LaunchMode::Persistent;
        }
        native_preloaded_config = Some(loaded);
    }

    #[cfg(windows)]
    if arguments.mode == LaunchMode::Persistent {
        if let Some(context) = runyte::workspace::parent::ParentContext::from_environment()? {
            let directory = std::env::current_dir()?;
            // With no selector the attachment uses the launch directory's own
            // workspace. Naming a directory may create one; this may not.
            let discovered =
                if arguments.workspace_selector.is_none() && arguments.project_root.is_none() {
                    let state = match native_preloaded_config.as_ref() {
                        Some((config, _)) => config.workspace.state.clone(),
                        None => Config::load(arguments.config.as_deref())?.0.workspace.state,
                    };
                    Some(
                        project_root::discover(&directory, &state)?
                            .context(project_root::NO_WORKSPACE_HERE)?,
                    )
                } else {
                    None
                };
            let selector = match (
                arguments.workspace_selector.as_deref(),
                arguments.project_root.as_deref(),
            ) {
                (Some(selector), _) => selector.to_path_buf(),
                (None, Some(root)) => resolve_requested_project_root(&directory, root)?,
                (None, None) => discovered.unwrap_or_else(|| directory.clone()),
            };
            let parent = ForegroundParentSupervisor::capture()?;
            report_retained_host_logging(&arguments);
            return tokio::select! {
                biased;
                event = native_termination.recv() => Err(terminated(event)),
                result = runyte::workspace::parent::run_attach(context, selector, directory, &parent) => result,
            };
        }
        return run_native_persistent(
            &arguments,
            startup,
            native_termination,
            native_preloaded_config,
        )
        .await;
    }

    #[cfg(windows)]
    if arguments.mode == LaunchMode::Wait
        && let Some(context) = runyte::workspace::parent::ParentContext::from_environment()?
    {
        anyhow::ensure!(
            arguments.project_root.is_none(),
            "--project-root is not available for a parent editor wait"
        );
        let directory = std::env::current_dir()?;
        let paths = arguments
            .targets
            .iter()
            .map(|target| {
                if target.path.is_absolute() {
                    target.path.clone()
                } else {
                    directory.join(&target.path)
                }
            })
            .collect();
        let parent = ForegroundParentSupervisor::capture()?;
        report_retained_host_logging(&arguments);
        return tokio::select! {
            biased;
            event = native_termination.recv() => Err(terminated(event)),
            result = runyte::workspace::parent::run_wait(context, paths, &parent) => result,
        };
    }

    #[cfg(windows)]
    if arguments.mode == LaunchMode::Wait {
        return run_native_wait(&arguments, startup, native_termination).await;
    }

    if arguments.mode == LaunchMode::ListContext {
        #[cfg(any(unix, windows))]
        {
            use runyte::workspace::context::{
                discovery,
                storage::{Storage, environment_fingerprint},
            };
            #[cfg(unix)]
            let root = Storage::default_root();
            #[cfg(windows)]
            let root = Storage::default_location().map(|location| location.root().to_owned());
            let result =
                discovery::discover(root, &environment_fingerprint(), arguments.include_hidden)
                    .await?;
            serde_json::to_writer(stdout().lock(), &result)?;
            println!();
            return Ok(());
        }
        #[cfg(all(not(unix), not(windows)))]
        anyhow::bail!("context discovery is not supported on this platform");
    }

    #[cfg(unix)]
    if matches!(arguments.mode, LaunchMode::Persistent | LaunchMode::Wait)
        && let Some(context) = runyte::workspace::parent::ParentContext::from_environment()?
    {
        return run_parent_request(&arguments, context, supervising_parent.as_ref()).await;
    }

    // The documented shell function adds `--cwd-file` to every invocation.
    // Modes without a directory-handoff-capable editor accept it and leave the
    // file untouched so session management remains transparent to the wrapper.
    if matches!(
        arguments.mode,
        LaunchMode::ListSessions
            | LaunchMode::StopAllSessions
            | LaunchMode::CleanSessions
            | LaunchMode::RenameSession
    ) || (matches!(
        arguments.mode,
        LaunchMode::RestartSession | LaunchMode::StopSession
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
                    list_sessions(&config.workspace.state, arguments.include_hidden).await
                }
                LaunchMode::StopAllSessions => {
                    let (config, config_path) = Config::load(arguments.config.as_deref())?;
                    stop_all_sessions(
                        &config.workspace.state,
                        config_path.as_deref(),
                        arguments.force,
                        arguments.include_hidden,
                    )
                    .await
                }
                LaunchMode::CleanSessions => {
                    let config = Config::load(arguments.config.as_deref())?.0;
                    let cleared = clear_stopped_sessions(&config.workspace.state).await?;
                    println!(
                        "forgot {cleared} stopped session{}",
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
        #[cfg(windows)]
        {
            return run_native_control_cli(&arguments, startup, native_termination).await;
        }
        #[cfg(all(not(unix), not(windows)))]
        anyhow::bail!("persistent mode is not yet supported on this platform");
    }

    #[cfg(windows)]
    if matches!(
        arguments.mode,
        LaunchMode::StopSession | LaunchMode::RestartSession
    ) {
        return run_native_control_cli(&arguments, startup, native_termination).await;
    }

    #[cfg(windows)]
    let (config, config_path) = match native_preloaded_config {
        Some(loaded) => loaded,
        None => Config::load(arguments.config.as_deref())?,
    };
    #[cfg(not(windows))]
    let (config, config_path) = Config::load(arguments.config.as_deref())?;
    let automatic_persistent = uses_automatic_persistent_mode(&arguments, config.mode());
    if automatic_persistent {
        #[cfg(unix)]
        {
            arguments.mode = LaunchMode::Persistent;
        }
        #[cfg(all(not(unix), not(windows)))]
        anyhow::bail!("mode: mux is not supported on this platform");
        #[cfg(windows)]
        unreachable!("automatic Windows persistent launch returned through native attachment");
    }
    startup.mark(StartupPhase::ConfigLoaded);
    // Decided before anything resolves a workspace: `-a DIRECTORY` may
    // create one, and as root that must not happen at all.
    let running_as_root = effective_user_is_root();
    let editor = launch_is_editor(&arguments, config.mode());
    refuse_workspace_modes_as_root(&arguments, editor, running_as_root)?;
    let launch_directory = std::env::current_dir()?;
    arguments.cwd_file = arguments
        .cwd_file
        .take()
        .map(|path| resolve_cwd_file_path(&launch_directory, path));
    // Only an editor that can perform the handoff admits the output path.
    // Pin its already-private parent before acquiring the terminal or editing.
    #[cfg(windows)]
    let cwd_handoff = if arguments.mode == LaunchMode::Standalone {
        arguments
            .cwd_file
            .as_deref()
            .map(runyte::cwd_handoff::Prepared::prepare)
            .transpose()
            .context("cannot prepare private PowerShell directory handoff")?
    } else {
        None
    };
    let mut reserved_user_roots = config_path
        .as_deref()
        .map(|path| config::config_root_for(path, &launch_directory))
        .into_iter()
        .collect::<Vec<_>>();
    if let Some(cache_root) = external_open::cache_root() {
        reserved_user_roots.push(cache_root);
    }
    // `--init` makes a workspace and stops there. Opening one is a separate
    // launch, so initializing never also decides how to edit in it.
    if let Some(requested) = arguments.init.take() {
        return initialize_workspace(
            &requested,
            &launch_directory,
            &config.workspace.state,
            &reserved_user_roots,
            &mut io::stdout().lock(),
        );
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
    let attaching_current = arguments.mode == LaunchMode::Persistent
        && arguments.mode_explicit
        && arguments.project_root.is_none()
        && !attaching_elsewhere;
    let project_root = match arguments.project_root.take() {
        // Editor mode has no workspace. The launch directory stands in as
        // the root that relative paths are displayed against; nothing
        // project-scoped reads it and its state directory is never made.
        _ if editor => {
            startup.mark(StartupPhase::ProjectResolvedAutomatically);
            launch_directory.clone()
        }
        _ if attaching_elsewhere => {
            let project_root = selected_workspace.expect("selector resolved a workspace");
            startup.mark(StartupPhase::ProjectResolvedAutomatically);
            project_root
        }
        // `-a` with no selector uses the workspace the launch directory
        // belongs to. Naming a directory creates one; this does not.
        _ if attaching_current => {
            let project_root = project_root::discover(&launch_directory, &config.workspace.state)?
                .context(project_root::NO_WORKSPACE_HERE)?;
            startup.mark(StartupPhase::ProjectResolvedAutomatically);
            project_root
        }
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
            None => anyhow::bail!(project_root::NO_WORKSPACE_HERE),
        },
    };
    let state_root = project_root::resolve_state_root(&project_root, &config.workspace.state);
    if !editor {
        project_root::validate_state_root(&state_root, &reserved_user_roots)?;
    }
    // A selected workspace does not contain the launch directory, so the host
    // it starts is given the workspace's own root. Handing it the directory the
    // shell was in would place a host outside the project it serves. Only the
    // Unix host launch below reads it.
    #[cfg(unix)]
    let working_directory = if attaching_elsewhere {
        project_root.clone()
    } else {
        launch_directory.clone()
    };
    #[cfg(unix)]
    let recorded_workspace = if editor {
        None
    } else if arguments.mode == LaunchMode::Standalone {
        record_recent_workspace(&project_root).ok().flatten()
    } else {
        ensure_recent_workspace(&project_root).ok().flatten()
    };
    let mouse_enabled = config.editor.mouse;
    if matches!(
        arguments.mode,
        LaunchMode::Persistent
            | LaunchMode::Wait
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
                    // launch under `mode: mux` has always
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
                        arguments.window,
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
                        supervising_parent
                            .as_ref()
                            .expect("wait launch records its parent"),
                    )
                    .await
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
    // Editor mode keeps no log unless one was asked for: its default
    // place is the state directory, which editor mode never creates.
    let logging_failure = if editor && arguments.log.is_none() {
        None
    } else {
        initialize_logging(
            &arguments,
            role,
            &state_root,
            (!editor).then_some(project_root.as_path()),
        )?
    };
    log_info!(
        "process",
        "runyte {} started", env!("CARGO_PKG_VERSION");
        "role" => role,
        "workspace" => if editor { "none".to_owned() } else { workspace_id(&project_root) },
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
    // A standalone process owns the terminal, so acquire it as soon as every
    // fallible launch decision that may need the ordinary terminal has been
    // made. In particular, do this before opening and parsing startup targets:
    // the deliberate startup presentation can be shown before its latency can
    // grow with document size or language work. The content frame still waits
    // for the complete highlighted editor state below, so no document text is
    // ever shown unhighlighted or reflowed after first appearing.
    //
    // Signal registration remains ahead of raw mode, and the guard stays live
    // across every later fallible step. An error therefore restores the
    // ordinary terminal before it is reported by `main`. A failed acquisition
    // is retained until editor and debug-trace construction finish, preserving
    // a more specific startup error on invocations that have no usable TTY.
    let standalone_color_depth =
        (arguments.mode == LaunchMode::Standalone).then(terminal_color_depth);
    // Unix preserves terminal state before installing its signal handler.
    // Windows already owns its console listeners from before launch parsing;
    // terminal restoration is handled by the guard during startup and exit.
    let startup_restore = if arguments.mode == LaunchMode::Standalone && !arguments.window {
        Some(StartupSignalExit::arm())
    } else {
        None
    };
    let mut standalone_termination = if arguments.window
        || startup_restore
            .as_ref()
            .is_some_and(std::result::Result::is_ok)
    {
        #[cfg(windows)]
        {
            Some(native_termination)
        }
        #[cfg(not(windows))]
        {
            Some(TerminationSignals::new()?)
        }
    } else {
        None
    };
    let mut startup_signal_exit = None;
    let standalone_terminal = if arguments.mode == LaunchMode::Standalone && !arguments.window {
        let terminal = match startup_restore.expect("standalone startup terminal state") {
            Ok(restore) => match TerminalGuard::enter(mouse_enabled) {
                Ok(guard) => {
                    startup.mark(StartupPhase::TerminalEntered);
                    match present_startup_screen() {
                        Ok(()) => {
                            startup.mark(StartupPhase::FirstFramePresented);
                            startup_signal_exit = Some(restore);
                            Ok(guard)
                        }
                        Err(error) => Err(error),
                    }
                }
                Err(error) => Err(error),
            },
            Err(error) => Err(error),
        };
        Some(terminal)
    } else {
        None
    };
    let native_targets = if arguments.window {
        std::mem::take(&mut arguments.targets)
    } else {
        Vec::new()
    };
    let mut app = if editor {
        App::new_editor_with_deferred_syntax(
            config,
            arguments.targets,
            project_root.clone(),
            startup,
        )?
    } else {
        App::new_in_project_with_deferred_syntax(
            config,
            arguments.targets,
            project_root.clone(),
            startup,
        )?
    };
    app.native_media = arguments.window;
    app.open_native_targets(native_targets)?;
    app.note_running_as_root(running_as_root);
    let startup_config = app.config.clone();
    app.note_config_deprecations(&startup_config);
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
    app.defer_frontend_presentation(arguments.window);

    if arguments.mode == LaunchMode::Serve {
        #[cfg(unix)]
        {
            if show_startup_about {
                app.app_mut().execute(about_invocation()?)?;
            }
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

    // The persistent host opens its own trace, so only a standalone editor
    // reaches this one.
    #[cfg(debug_assertions)]
    let mut input_trace = open_input_trace()?;
    // The standalone resources were acquired before editor construction. Move
    // them into the interactive loop now that the persistent-host branch has
    // returned.
    let color_depth = if arguments.window {
        ui::TerminalColorDepth::TrueColor
    } else {
        standalone_color_depth.expect("standalone terminal colour depth")
    };
    let _terminal = standalone_terminal.transpose()?;
    #[cfg(windows)]
    let termination = standalone_termination
        .take()
        .expect("standalone termination signals");
    #[cfg(not(windows))]
    let mut termination = standalone_termination
        .take()
        .expect("standalone termination signals");
    let backend = CrosstermBackend::new(stdout());
    #[cfg(not(feature = "native"))]
    let mut terminal = Terminal::new(backend)?;
    #[cfg(feature = "native")]
    let mut terminal = native_frontend::Surface::new(backend, arguments.window)?;
    let mut received_signal = None;
    let mut key_hints = KeyHintState::default();
    if show_startup_about {
        app.app_mut().execute(about_invocation()?)?;
    }
    terminal.draw(|frame| {
        let geometry = ui::frame_geometry(frame.area());
        #[cfg(feature = "native")]
        native_frontend::update_media(app.app_mut());
        let snapshot = app.prepare_frame_with_hints(geometry, Some(&key_hints));
        #[cfg(feature = "native")]
        native_frontend::capture_media(&snapshot, app.app_mut(), &key_hints);
        #[cfg(feature = "native")]
        if arguments.window {
            native_frontend::render_frame(frame, app.app(), &snapshot, color_depth);
        } else {
            ui::render(frame, app.app(), &snapshot.editor, &key_hints, color_depth);
        }
        #[cfg(not(feature = "native"))]
        ui::render(frame, app.app(), &snapshot.editor, &key_hints, color_depth);
    })?;
    startup.mark(StartupPhase::EditorFramePresented);
    if let Err(error) = startup.write_requested() {
        app.report_host_error(format!("failed to write startup timing report: {error}"));
    }

    // Optional services start only after the standalone editor is usable.
    // Their initialization must never hide first-frame latency.
    // The native catalog describes the workspace this editor serves, and a
    // session in editor mode serves none.
    #[cfg(windows)]
    let native_catalog = if editor {
        None
    } else {
        standalone_native_catalog_config(&mut app, &reserved_user_roots)
    };
    let mut services = start_host_services(
        &mut app,
        startup,
        config_path.as_deref(),
        false,
        #[cfg(windows)]
        native_catalog,
    )?;
    if let Err(error) = startup.write_requested() {
        app.report_host_error(format!("failed to write startup timing report: {error}"));
    }
    let interactive_outcome: Result<()> = async {
    #[cfg(all(windows, debug_assertions))]
    wait_at_post_service_failure_barrier().await?;
    // Service discovery can add a useful failure/status message. Present it
    // before waiting for input so a quiet terminal never leaves the initial
    // pre-service frame stale. From this point onward every fallible frontend
    // setup step is captured so the services below are joined during cleanup.
    terminal.draw(|frame| {
        let geometry = ui::frame_geometry(frame.area());
        #[cfg(feature = "native")]
        native_frontend::update_media(app.app_mut());
        let snapshot = app.prepare_frame_with_hints(geometry, Some(&key_hints));
        #[cfg(feature = "native")]
        native_frontend::capture_media(&snapshot, app.app_mut(), &key_hints);
        #[cfg(feature = "native")]
        if arguments.window {
            native_frontend::render_frame(frame, app.app(), &snapshot, color_depth);
        } else {
            ui::render(frame, app.app(), &snapshot.editor, &key_hints, color_depth);
        }
        #[cfg(not(feature = "native"))]
        ui::render(frame, app.app(), &snapshot.editor, &key_hints, color_depth);
    })?;
    // This is a signal-restoration guard on Unix and a unit value elsewhere.
    #[allow(clippy::drop_non_drop)]
    drop(startup_signal_exit.take());
    #[cfg(not(windows))]
    let mut terminal_events = {
        #[cfg(feature = "native")]
        { native_frontend::Events::new(arguments.window) }
        #[cfg(not(feature = "native"))]
        { EventStream::new() }
    };
    #[cfg(windows)]
    let mut terminal_events = runyte::tui::windows_input::EventStream::new()?;
    let mut git_refresh_tick = tokio::time::interval(MAINTENANCE_INTERVAL);
    git_refresh_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut status_animation_tick = tokio::time::interval(STATUS_ANIMATION_INTERVAL);
    status_animation_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut frame_tick = tokio::time::interval(FRAME_INTERVAL);
    frame_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    // Set by the branches whose state changes faster than a reader can follow.
    // Every other branch falls through to the draw below, which clears it.
    let mut frame_pending = false;
    let mut finder_refresh_tick = tokio::time::interval(FINDER_TERMINAL_REFRESH_INTERVAL);
    finder_refresh_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut key_repeat_detector = KeyRepeatDetector::default();
    // Recorded once per service: a channel that closes stays closed, and the
    // editor keeps working without it, so nothing else reports the loss.
    let mut ended_services: std::collections::HashSet<&'static str> =
        std::collections::HashSet::new();
    let directory_tree_wake = app.app().directory_tree.wake();
    loop {
        key_hints.expire_at(Instant::now());
        if app.should_quit {
            break;
        }
        let hint_timeout = key_hints.time_until_expiry(Instant::now());
        let picker_pacing = app.picker_pacing_delay(Instant::now());
        let pointer_autoscroll = app.pointer_autoscroll_delay(Instant::now());
        #[cfg(all(feature = "native", not(windows)))]
        let mut applied_key_or_text = false;
        app.note_plugin_frontend(true);
        app.sync_plugin_observers();
        app.sync_context();
        let context_delay = app.context_delay();
        tokio::select! {
            _ = directory_tree_wake.notified() => {
                if !app.app_mut().directory_tree.poll() {
                    continue;
                }
            }
            Some(event) = services.context_events.recv() => { app.handle_context_event(event); if !app.plugin_presentation_pending() { continue; } }
            _ = context_timeout(context_delay) => { app.sync_context(); if !app.plugin_presentation_pending() { continue; } }
            _ = std::future::ready(()), if app.plugin_presentation_pending() => { app.take_plugin_presentation_change(); }
            Some(event) = services.pipe_events.recv() => { app.handle_pipe_completion(event); }
            event = runyte::plugin::receive(&mut services.plugin_events) => {
                if let Some(event) = event {
                    if !app.handle_plugin_event(event) && !app.plugin_presentation_pending() {
                        continue;
                    }
                } else {
                    services.plugin_events = None;
                    continue;
                }
            }
            input = terminal_events.next() => {
                #[cfg(all(feature = "native", not(windows)))]
                if terminal_events.is_presentation_acknowledgement() {
                    app.acknowledge_frontend_paint(terminal_events.presented_frame(None));
                    continue;
                }
                #[cfg(feature = "native")]
                if native_frontend::take_close_request()
                    && let Err(error) = app.app_mut().execute(runyte::command::parse_colon_command("qa")?) {
                    app.report_host_error(error.to_string());
                }
                match input.transpose()? {
                    // Fall through to the draw at the bottom of the loop
                    // rather than taking the lifecycle `continue` below.
                    Some(event) if is_redraw_only_event(&event) => {
                        key_repeat_detector.observe(None, None, Instant::now());
                    }
                    Some(event) => {
                        let key_kind = terminal_key_kind(&event);
                        #[cfg(windows)]
                        let converted = convert_windows_event(event, app.app())?;
                        #[cfg(not(windows))]
                        let converted = convert_event(event)?;
                        let Some(input) = converted else {
                            key_repeat_detector.observe(key_kind, None, Instant::now());
                            continue;
                        };
                        let repeated = key_repeat_detector.observe(
                            key_kind,
                            Some(&input),
                            Instant::now(),
                        );
                        #[cfg(all(feature = "native", not(windows)))]
                        let input_frame = terminal_events.presented_frame(app.current_frame_id());
                        #[cfg(any(not(feature = "native"), windows))]
                        let input_frame = app.current_frame_id();
                        #[cfg(all(feature = "native", not(windows)))]
                        if !terminal_events.accepts_input(&app, &input, frame_pending || app.finder_scan_refills() || app.plugin_presentation_pending()) {
                            continue;
                        }
                        app.acknowledge_frontend_input_frame(input_frame);
                        if let Some(frame) = input_frame { app.context_frame_presented(frame); }
                        if let Some(message) = rejected_text_input(&input) {
                            app.report_host_error(message);
                            if frame_publication_ready(
                                true,
                                app.finder_scan_refills(),
                                &mut frame_pending,
                            ) {
                                terminal.draw(|frame| {
                                    let geometry = ui::frame_geometry(frame.area());
                                    #[cfg(feature = "native")]
        native_frontend::update_media(app.app_mut());
        let snapshot = app.prepare_frame_with_hints(
                                        geometry,
                                        Some(&key_hints),
                                    );
                                    #[cfg(feature = "native")]
        native_frontend::capture_media(&snapshot, app.app_mut(), &key_hints);
        #[cfg(feature = "native")]
        if arguments.window {
            native_frontend::render_frame(frame, app.app(), &snapshot, color_depth);
        } else {
            ui::render(frame, app.app(), &snapshot.editor, &key_hints, color_depth);
        }
        #[cfg(not(feature = "native"))]
        ui::render(frame, app.app(), &snapshot.editor, &key_hints, color_depth);
                                })?;
                                frame_pending = false;
                            }
                            continue;
                        }
                        #[cfg(debug_assertions)]
                        let sensitive_input = app.app().plugin_input_active();
                        #[cfg(debug_assertions)]
                        trace_input(
                            input_trace.as_mut(),
                            "before",
                            app.app(),
                            &input,
                            sensitive_input,
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
                                if let Some(frame) = input_frame {
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
                            InputEvent::Key(_) | InputEvent::Text(_) | InputEvent::ClipboardPaste => {
                                observe_key_or_text_hint(app.app(), &mut key_hints, &input)
                            }
                        };
                        if hint_result == HintEventResult::Forward {
                            let dispatches = motion_repeat_dispatches(&app, &input, repeated);
                            for _ in 0..dispatches {
                                let result = app.execute_frontend_input(input.clone(), repeated);
                                if let Err(error) = result {
                                    app.report_host_error(error.to_string());
                                    break;
                                }
                            }
                        }
                        #[cfg(all(feature = "native", not(windows)))]
                        {
                            applied_key_or_text = matches!(
                                input,
                                InputEvent::Key(_) | InputEvent::Text(_) | InputEvent::ClipboardPaste
                            );
                        }
                        #[cfg(debug_assertions)]
                        trace_input(
                            input_trace.as_mut(),
                            "after",
                            app.app(),
                            &input,
                            sensitive_input,
                            repeated,
                            Some(hint_result),
                        )?;
                    }
                    None => break,
                }
            }
            event = receive_service_event("language servers", &ended_services, services.lsp_events.recv()) => {
                if let Some(event) = event {
                    app.apply_event(HostEvent::Lsp(event));
                } else {
                    note_ended_service(&mut ended_services, "language servers");
                }
            }
            event = services.syntax_events.recv(), if !ended_services.contains("syntax") => {
                if let Some(event) = event {
                    app.apply_event(HostEvent::Syntax(event));
                    #[cfg(feature = "startup-timing")]
                    if app.syntax.first().is_some_and(Option::is_some)
                        && startup.note_initial_syntax_ready() {
                        if let Err(error) = startup.write_requested() {
                            app.report_host_error(format!("failed to write startup timing report: {error}"));
                        }
                    }
                } else {
                    app.syntax_worker_stopped();
                    note_ended_service(&mut ended_services, "syntax");
                }
            }
            event = receive_service_event("file picker", &ended_services, services.file_picker_events.recv()) => {
                if let Some(event) = event {
                    let paced = pace_file_picker_event(&event);
                    app.apply_event(HostEvent::FilePicker(event));
                    if paced {
                        frame_pending = true;
                        continue;
                    }
                } else {
                    note_ended_service(&mut ended_services, "file picker");
                }
            }
            event = receive_service_event("workspace search", &ended_services, services.workspace_search_events.recv()) => {
                if let Some(event) = event {
                    app.apply_event(HostEvent::WorkspaceSearch(event));
                } else {
                    note_ended_service(&mut ended_services, "workspace search");
                }
            }
            event = receive_service_event("file monitor", &ended_services, services.file_monitor_events.recv()) => {
                if let Some(event) = event {
                    app.apply_event(HostEvent::FileObservation(event));
                } else {
                    note_ended_service(&mut ended_services, "file monitor");
                }
            }
            event = receive_service_event("Git monitor", &ended_services, services.git_monitor_events.recv()) => {
                if let Some(event) = event {
                    app.apply_event(HostEvent::GitInvalidation(event));
                    let _ = app.refresh_git_if_due(Instant::now());
                } else {
                    note_ended_service(&mut ended_services, "Git monitor");
                }
            }
            output = services.terminal_events.recv() => {
                if let Some(output) = output {
                    app.apply_event(HostEvent::Terminal(output));
                    terminal::drain(&mut services.terminal_events, |output| {
                        app.apply_event(HostEvent::Terminal(output));
                    });
                    frame_pending = true;
                    continue;
                }
            }
            event = receive_optional_service_event(&mut services.workspace_events) => {
                if let Some(event) = event {
                    app.apply_event(event);
                }
            }
            event = async {
                #[cfg(windows)]
                { match services.native_catalog_events.as_mut() {
                    Some(events) => events.recv().await,
                    None => std::future::pending().await,
                }}
                #[cfg(not(windows))]
                { std::future::pending::<Option<()>>().await }
            } => {
                #[cfg(windows)]
                if let Some(event) = event {
                    app.apply_event(HostEvent::Workspace(event));
                } else {
                    services.native_catalog_events = None;
                    app.app_mut().detach_workspace_service();
                    note_ended_service(&mut ended_services, "native session catalog");
                }
                #[cfg(not(windows))]
                let _ = event;
            }
            event = receive_optional_service_event(&mut services.git_events) => {
                if let Some(event) = event {
                    app.apply_event(HostEvent::Git(event));
                }
            }
            _ = git_refresh_tick.tick() => {
                report_logging_failure(app.app_mut());
                services.file_monitor.sync(app.file_monitor_requests());
                services.git_monitor.sync(app.git_monitor_repository());
                let changed = app.refresh_git_if_due(Instant::now());
                let activity_changed = app.refresh_session_activity();
                let open_changed = app.app_mut().poll_external_opens(Instant::now());
                if !changed && !activity_changed && !open_changed {
                    continue;
                }
            }
            _ = tokio::time::sleep(pointer_autoscroll.unwrap_or_default()), if pointer_autoscroll.is_some() => {
                if !app.advance_pointer_autoscroll(Instant::now()) {
                    continue;
                }
            }
            _ = status_animation_tick.tick(), if app.has_long_running_action() => {}
            _ = tokio::task::yield_now(), if app.macro_replay_pending() => {
                if let Err(error) = app.advance_macro_replay() {
                    app.report_host_error(error.to_string());
                }
            }
            _ = finder_refresh_tick.tick(), if app.finder_terminals_dirty() => {
                // A content refresh drops the rows it is about to read back,
                // so this state is a hole rather than an answer. The pass that
                // refills it decides when there is a frame worth drawing.
                if !app.refresh_finder_terminals() || app.resource_finder_scan_pending() {
                    continue;
                }
            }
            _ = tokio::task::yield_now(), if app.resource_finder_scan_pending() => {
                app.advance_resource_finder_scan();
                // A slice is one of many states a pass moves through; only the
                // one that ends it is worth a frame of its own. The rest wait
                // for the frame tick, which holds them back entirely while a
                // refresh is refilling.
                if app.resource_finder_scan_pending() {
                    frame_pending = true;
                    continue;
                }
            }
            // Nothing a refill passes through is worth drawing: between
            // dropping a terminal's rows and finding them again the list has a
            // hole where results the reader was looking at used to be.
            _ = frame_tick.tick(), if (frame_pending || app.diff_work_pending()) && !app.finder_scan_refills() => {
                if !frame_pending && !app.poll_diff_work() { continue; }
            }
            // Paced picker state comes due without an event to carry it: a
            // ranked answer waiting for the rows to age out, and header
            // counts the work stopped short of. Nothing else would wake for
            // either, so the loop sleeps until whichever is nearer — and,
            // like a frame, neither is published through a refill.
            _ = tokio::time::sleep(picker_pacing.unwrap_or_default()),
                if picker_pacing.is_some() && !app.finder_scan_refills() =>
            {
                app.advance_picker_pacing();
            }
            _ = tokio::time::sleep(hint_timeout.unwrap_or_default()), if hint_timeout.is_some() => {
                key_hints.expire_at(Instant::now());
            }
            signal = termination.recv() => {
                received_signal = Some(signal);
                break;
            }
        }
        #[cfg(all(feature = "native", not(windows)))]
        if terminal_events.defer_frame(applied_key_or_text, &mut frame_pending) {
            continue;
        }
        if !frame_publication_ready(true, app.finder_scan_refills(), &mut frame_pending) {
            continue;
        }
        terminal.draw(|frame| {
            let geometry = ui::frame_geometry(frame.area());
            #[cfg(feature = "native")]
        native_frontend::update_media(app.app_mut());
        let snapshot = app.prepare_frame_with_hints(geometry, Some(&key_hints));
            #[cfg(feature = "native")]
        native_frontend::capture_media(&snapshot, app.app_mut(), &key_hints);
        #[cfg(feature = "native")]
        if arguments.window {
            native_frontend::render_frame(frame, app.app(), &snapshot, color_depth);
        } else {
            ui::render(frame, app.app(), &snapshot.editor, &key_hints, color_depth);
        }
        #[cfg(not(feature = "native"))]
        ui::render(frame, app.app(), &snapshot.editor, &key_hints, color_depth);
        })?;
        frame_pending = false;
    }
    Ok(())
    }.await;
    #[cfg(not(windows))]
    interactive_outcome?;
    let quit_directory = app.quit_directory().map(Path::to_path_buf);
    services.language_servers.send(LspCommand::Shutdown);
    #[cfg(windows)]
    let context_shutdown = app.shutdown_context().await.map_err(anyhow::Error::from);
    #[cfg(windows)]
    let catalog_shutdown = services.shutdown_native_catalog().await;
    let plugins_shutdown = app.shutdown_plugins().await;
    #[cfg(windows)]
    finish_standalone_native(
        interactive_outcome,
        context_shutdown,
        catalog_shutdown,
        plugins_shutdown,
        received_signal,
    )?;
    #[cfg(not(windows))]
    plugins_shutdown?;
    #[cfg(not(windows))]
    if let (Some(cwd_file), Some(directory)) = (arguments.cwd_file.as_deref(), quit_directory) {
        write_cwd_file(cwd_file, &directory)?;
    }
    #[cfg(windows)]
    if let (Some(handoff), Some(directory)) = (cwd_handoff.as_ref(), quit_directory) {
        handoff
            .write(&directory)
            .context("cannot publish PowerShell directory handoff")?;
    }
    #[cfg(not(windows))]
    if let Some(signal) = received_signal {
        return Err(terminated(signal));
    }
    Ok(())
}

/// Whether this process should open the front page before its first frame.
///
/// A targetless standalone launch is the original case. A workspace host is
/// the same launch seen from the other side of the transport: it is started
/// without targets, and the first client to attach finds whatever state the
/// host began with. Opening the page there rather than on attachment keeps it
/// a property of a new, empty session, so detaching and attaching again does
/// not bring back a page the reader has already replaced.
fn starts_on_about(arguments: &LaunchArguments) -> bool {
    matches!(arguments.mode, LaunchMode::Standalone | LaunchMode::Serve)
        && arguments.targets.is_empty()
}

/// The `:about` invocation a launch that starts on the front page runs.
fn about_invocation() -> Result<CommandInvocation, CommandInvocationError> {
    CommandInvocation::editor(EditorCommand::ShowAbout, CommandExecutionContext::default())
}

fn uses_automatic_persistent_mode(arguments: &LaunchArguments, configured: RunMode) -> bool {
    // The persistent default is deliberately a bare-launch convenience. A
    // target may carry a caller-relative path or an initial caret position,
    // and the attach protocol does not represent all of those launch
    // semantics. Keep target-bearing invocations on the ordinary standalone
    // path unless a future protocol can preserve the complete target.
    !arguments.window
        && !arguments.mode_explicit
        && arguments.targets.is_empty()
        && arguments.init.is_none()
        && !arguments.editor
        && configured == RunMode::Mux
}

/// Whether this launch runs in editor mode: standalone, with no workspace.
/// `--editor` asks for it, and so does `mode: editor` when no mode option is
/// given and nothing names a workspace; the launch directory never decides.
fn launch_is_editor(arguments: &LaunchArguments, configured: RunMode) -> bool {
    arguments.mode == LaunchMode::Standalone
        && (arguments.editor
            || (!arguments.mode_explicit
                && configured == RunMode::Editor
                && arguments.project_root.is_none()
                && arguments.init.is_none()))
}

/// `runyte --init DIRECTORY`: makes exactly `DIRECTORY` a workspace, or
/// accepts one that already is, and says how to open it.
fn initialize_workspace(
    requested: &Path,
    launch_directory: &Path,
    configured_state: &Path,
    reserved_user_roots: &[PathBuf],
    output: &mut impl Write,
) -> Result<()> {
    let requested = if requested.is_absolute() {
        requested.to_path_buf()
    } else {
        launch_directory.join(requested)
    };
    let existed = requested
        .canonicalize()
        .is_ok_and(|root| project_root::resolve_state_root(&root, configured_state).is_dir());
    let project_root = project_root::initialize(&requested, configured_state, reserved_user_roots)?;
    let root = project_root.display();
    if existed {
        writeln!(output, "{root} is already a workspace")?;
    } else {
        writeln!(output, "initialized a workspace in {root}")?;
    }
    writeln!(
        output,
        "open it with: runyte {root}   or   runyte --mux {root}"
    )?;
    Ok(())
}

/// Refuses every mode but editor as root. `ide`, `mux`, `--wait`, `--init`
/// and a foreground host all belong to a workspace, and as root they would
/// leave root-owned runtime state in a workspace some other account owns.
fn refuse_workspace_modes_as_root(
    arguments: &LaunchArguments,
    editor: bool,
    running_as_root: bool,
) -> Result<()> {
    let workspace_mode = match arguments.mode {
        // `--init` opens nothing, but the state directory it creates would be
        // root's inside a project another account owns.
        LaunchMode::Standalone => !editor,
        LaunchMode::Persistent | LaunchMode::Wait | LaunchMode::Serve => true,
        _ => false,
    };
    anyhow::ensure!(
        !(running_as_root && workspace_mode),
        "as root, runyte runs only in editor mode; use runed FILE, or sudoedit FILE"
    );
    Ok(())
}

/// Whether the editor process runs with root's effective user ID.
fn effective_user_is_root() -> bool {
    #[cfg(unix)]
    {
        // SAFETY: geteuid has no preconditions.
        unsafe { libc::geteuid() == 0 }
    }
    #[cfg(not(unix))]
    {
        false
    }
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
#[cfg_attr(not(unix), allow(dead_code))]
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

#[cfg_attr(not(unix), allow(dead_code))]
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
    let canonical_launch = launch_directory.canonicalize().with_context(|| {
        format!(
            "cannot resolve launch directory {}",
            launch_directory.display()
        )
    })?;
    let project_root = requested
        .canonicalize()
        .with_context(|| format!("cannot resolve project root {}", requested.display()))?;
    anyhow::ensure!(
        project_root.is_dir(),
        "project root {} is not a directory",
        project_root.display()
    );
    anyhow::ensure!(
        canonical_launch.starts_with(&project_root),
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
    #[cfg(debug_assertions)]
    let mut input_trace = open_input_trace()?;
    host.enable_persistent_session();
    let mut server = LocalServer::bind(&endpoint).await?;
    host.app_mut()
        .terminals
        .set_parent_launch(runyte::workspace::parent::ParentLaunch::new(
            endpoint.metadata(),
        )?);
    log_info!(
        "host",
        "persistent session published";
        "workspace" => endpoint.id(),
        "socket" => endpoint.socket().display()
    );
    let mut services = start_host_services(&mut host, startup, config_path, true)?;
    let mut last_detached = Instant::now();
    let mut idle_tick = tokio::time::interval(Duration::from_secs(1));
    idle_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut active: Option<AttachedClient> = None;
    let mut peer_processes = std::collections::HashMap::new();
    let mut control_attachments = std::collections::HashMap::new();
    let mut parent_switches: std::collections::HashMap<String, (u64, Instant)> =
        std::collections::HashMap::new();
    let mut controls: std::collections::HashMap<u64, runyte::workspace::transport::ResponseSender> =
        std::collections::HashMap::new();
    let mut control_wait_tokens: std::collections::HashMap<u64, Vec<WaitToken>> =
        std::collections::HashMap::new();
    let mut refresh_tick = tokio::time::interval(MAINTENANCE_INTERVAL);
    refresh_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut status_animation_tick = tokio::time::interval(STATUS_ANIMATION_INTERVAL);
    status_animation_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut frame_tick = tokio::time::interval(FRAME_INTERVAL);
    frame_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut frame_pending = false;
    let mut finder_refresh_tick = tokio::time::interval(FINDER_TERMINAL_REFRESH_INTERVAL);
    finder_refresh_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut key_hints = KeyHintState::default();
    let mut shutting_down = false;
    let mut received_signal = None;
    // A background service whose channel closes is gone for the rest of this
    // host's life, and a detached host has no other way to say so.
    let mut ended_services: std::collections::HashSet<&'static str> =
        std::collections::HashSet::new();
    let directory_tree_wake = host.app().directory_tree.wake();
    while !shutting_down {
        key_hints.expire_at(Instant::now());
        let mut changed = false;
        let hint_timeout = key_hints.time_until_expiry(Instant::now());
        let picker_pacing = host.picker_pacing_delay(Instant::now());
        if active.is_none() {
            host.cancel_pointer_drag();
            if host.app().native_media {
                host.app_mut().native_media = false;
                host.app_mut().media_requests.clear();
                host.defer_frontend_presentation(false);
            }
        }
        let pointer_autoscroll = host.pointer_autoscroll_delay(Instant::now());
        host.note_plugin_frontend(active.is_some());
        host.sync_plugin_observers();
        host.sync_context();
        let context_delay = host.context_delay();
        tokio::select! {
            _ = directory_tree_wake.notified() => {
                changed = host.app_mut().directory_tree.poll();
            }
            Some(event) = services.context_events.recv() => { host.handle_context_event(event); changed |= host.plugin_presentation_pending(); }
            _ = context_timeout(context_delay) => { host.sync_context(); changed |= host.plugin_presentation_pending(); }
            _ = std::future::ready(()), if host.plugin_presentation_pending() => { changed = host.take_plugin_presentation_change(); }
            Some(event) = services.pipe_events.recv() => { host.handle_pipe_completion(event); changed = true; }
            event = runyte::plugin::receive(&mut services.plugin_events) => {
                if let Some(event) = event { changed |= host.handle_plugin_event(event); }
                else { services.plugin_events = None; }
            }
            event = server.recv() => {
                let Some(event) = event else {
                    log_error!("host", "the connection listener stopped; this session cannot be reached again");
                    anyhow::bail!("workspace host listener stopped unexpectedly");
                };
                match event {
                    ServerEvent::Connected { id, peer_process, geometry, interactive, native_media, directory_handoff, responses } => {
                        peer_processes.insert(id, peer_process);
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
                            host.app_mut().native_media = native_media;
                            host.app_mut().media_requests.clear();
                            host.defer_frontend_presentation(native_media);
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
                                if frame_publication_ready(
                                    true,
                                    host.finder_scan_refills(),
                                    &mut frame_pending,
                                ) {
                                    #[cfg_attr(not(debug_assertions), allow(unused_variables))]
                                    let published =
                                        publish_attached_frame(&mut host, &mut active, &key_hints);
                                    #[cfg(debug_assertions)]
                                    trace_host_event(
                                        input_trace.as_mut(),
                                        format_args!("published {published:?} on attach"),
                                    )?;
                                    frame_pending = false;
                                }
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
                            control_attachments.insert(id, active.as_ref().map(|client| client.id));
                            controls.insert(id, responses);
                            control_wait_tokens.insert(id, Vec::new());
                        }
                    }
                    ServerEvent::Request { id, request } => {
                        let interactive = active.as_ref().is_some_and(|client| client.id == id);
                        let control = controls.contains_key(&id);
                        if !interactive && !control {
                            continue;
                        }
                        if control {
                            if let ClientRequest::ParentAttach { terminal, capability, selector, directory } = &request {
                                let terminal = runyte::terminal::TerminalId::from_raw(*terminal);
                                let valid = active.as_ref().is_some_and(|client| control_attachments.get(&id).copied().flatten() == Some(client.id)) && host.app().active_terminal() == Some(terminal) && host.parent_request_ready()
                                    && host.app().terminals.validates_parent(terminal, capability, peer_processes.get(&id).copied().flatten());
                                if !valid || !parent_switches.is_empty() {
                                    send_control_response(&mut controls, id, HostResponse::Error { message:
                                        "parent attachment is stale, detached, busy, or not owned by this terminal; return to its session and dismiss any overlay".to_owned() });
                                } else {
                                    let receipt = runyte::hash::sha256_hex(format!("{}:{}:{:?}", capability, id, Instant::now()).as_bytes());
                                    parent_switches.insert(receipt.clone(), (id, Instant::now()));
                                    send_active_response(&mut active, HostResponse::ParentSwitchWorkspace {
                                        selector: selector.clone(), directory: directory.clone(), receipt: receipt.clone() });
                                    if active.is_none() {
                                        parent_switches.remove(&receipt);
                                        send_control_response(&mut controls, id, HostResponse::Error { message:
                                            "outer TUI could not receive the attachment request; reconnect it before retrying".to_owned() });
                                    }
                                    active = None;
                                    last_detached = Instant::now();
                                }
                            } else if let ClientRequest::ParentHandoffResult { receipt, error } = &request {
                                if let Some((child, _)) = parent_switches.remove(receipt) {
                                    let response = error.as_ref().map_or(HostResponse::ParentAttached,
                                        |message| HostResponse::Error { message: message.clone() });
                                    send_control_response(&mut controls, child, response);
                                    send_control_response(&mut controls, id, HostResponse::ParentAttached);
                                } else {
                                    send_control_response(&mut controls, id, HostResponse::Error { message: "parent handoff expired or already completed".to_owned() });
                                }
                            } else if let ClientRequest::ParentWait { terminal, capability, paths } = &request {
                                let terminal = runyte::terminal::TerminalId::from_raw(*terminal);
                                let result = if active.as_ref().is_some_and(|client| control_attachments.get(&id).copied().flatten() == Some(client.id)) && host.app().terminals.validates_parent(terminal, capability,
                                    peer_processes.get(&id).copied().flatten()) {
                                    paths.iter().cloned().map(decode_path).collect::<io::Result<Vec<_>>>()
                                        .map_err(anyhow::Error::from)
                                        .and_then(|paths| host.create_parent_wait_request(terminal, paths))
                                } else { Err(anyhow::anyhow!("parent editing context is stale or detached; return to the owning persistent session")) };
                                let response = result.map_or_else(|error| HostResponse::Error { message: error.to_string() }, |(token,buffers)| {
                                    let token: WaitToken = token.into();
                                    control_wait_tokens.entry(id).or_default().push(token);
                                    HostResponse::WaitCreated { token, buffers: buffers.into_iter().map(Into::into).collect(), interactive_attached: true }
                                });
                                changed |= matches!(response, HostResponse::WaitCreated { .. });
                                send_control_response(&mut controls, id, response);
                            } else if matches!(request, ClientRequest::Shutdown) {
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
                                    && let Some(tokens) = control_wait_tokens.get_mut(&id)
                                    && !tokens.contains(token)
                                {
                                    tokens.push(*token);
                                }
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
                            ClientRequest::FrameDrawn { frame } => {
                                host.acknowledge_frontend_paint(Some(frame.into()));
                            }
                            ClientRequest::MediaNavigate { pane, path, delta } if host.app().native_media => {
                                host.app_mut().navigate_native_media(pane, &decode_path(path)?, delta);
                                changed = true;
                            }
                            ClientRequest::MediaBack { pane, path, page } if host.app().native_media => {
                                host.app_mut().leave_native_media(pane, &decode_path(path)?, page);
                                changed = true;
                            }
                            ClientRequest::MediaPages { path, pages } if host.app().native_media => {
                                host.app_mut().update_native_media_pages(&decode_path(path)?, pages);
                                changed = true;
                            }
                            ClientRequest::MediaNavigate { .. } | ClientRequest::MediaBack { .. } | ClientRequest::MediaPages { .. } => {}
                            ClientRequest::Input { event, repeated, presented_frame } => {
                                let input: InputEvent = event.clone().into();
                                if !host.accepts_frontend_input(&input, presented_frame.map(Into::into), frame_pending || host.finder_scan_refills() || host.plugin_presentation_pending()) { continue; }
                                host.acknowledge_frontend_input_frame(presented_frame.map(Into::into));
                                if !repeated && let Some(frame) = presented_frame { host.context_frame_presented(frame.into()); }
                                let input: InputEvent = event.into();
                                #[cfg(debug_assertions)]
                                let sensitive_input = host.app().plugin_input_active();
                                #[cfg(debug_assertions)]
                                let phase = format!(
                                    "host connection={id} presented={:?}",
                                    presented_frame.map(|frame| frame.get())
                                );
                                #[cfg(debug_assertions)]
                                trace_input(
                                    input_trace.as_mut(),
                                    &format!("{phase} before"),
                                    host.app(),
                                    &input,
                                    sensitive_input,
                                    repeated,
                                    None,
                                )?;
                                #[cfg_attr(not(debug_assertions), allow(unused_variables))]
                                let hint_result = dispatch_host_key_or_text(
                                    &mut host,
                                    &mut key_hints,
                                    input.clone(),
                                    repeated,
                                );
                                #[cfg(debug_assertions)]
                                trace_input(
                                    input_trace.as_mut(),
                                    &format!("{phase} after"),
                                    host.app(),
                                    &input,
                                    sensitive_input,
                                    repeated,
                                    Some(hint_result),
                                )?;
                                host.reconcile_wait_requests();
                                changed = true;
                            }
                            ClientRequest::Pointer {
                                event,
                                frame,
                                repetitions,
                            } => {
                                let input = InputEvent::Pointer(event.into());
                                if !host.accepts_frontend_input(&input, Some(frame.into()), frame_pending || host.finder_scan_refills() || host.plugin_presentation_pending()) { continue; }
                                host.acknowledge_frontend_input_frame(Some(frame.into()));
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
                            request @ (ClientRequest::NativeSwitchCommit { .. }
                            | ClientRequest::NativeParentSwitchCommitObserved { .. }
                            | ClientRequest::NativeSwitchAbort { .. }) => {
                                send_active_response(
                                    &mut active,
                                    unix_native_switch_refusal(&request)
                                        .expect("provisional switch request was matched"),
                                );
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
                            | ClientRequest::RenameHost { .. }
                            | ClientRequest::DestinationInventory
                            | ClientRequest::VisitDestination { .. }
                            | ClientRequest::ParentAttach { .. }
                            | ClientRequest::ParentWait { .. }
                            | ClientRequest::ParentHandoffResult { .. } => {}
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
                        peer_processes.remove(&id);
                        control_attachments.remove(&id);
                        let control = controls.remove(&id).is_some()
                            || control_wait_tokens.contains_key(&id);
                        for token in control_wait_tokens.remove(&id).unwrap_or_default() {
                            let _ = host.cancel_wait(
                                token.into(),
                                "wait client disconnected before completion",
                            );
                        }
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
            event = receive_service_event("language servers", &ended_services, services.lsp_events.recv()) => {
                if let Some(event) = event {
                    host.apply_event(HostEvent::Lsp(event));
                    changed = true;
                } else {
                    note_ended_service(&mut ended_services, "language servers");
                }
            }
            event = services.syntax_events.recv(), if !ended_services.contains("syntax") => {
                if let Some(event) = event {
                    host.apply_event(HostEvent::Syntax(event));
                    #[cfg(feature = "startup-timing")]
                    if host.syntax.first().is_some_and(Option::is_some)
                        && startup.note_initial_syntax_ready() {
                        if let Err(error) = startup.write_requested() {
                            host.report_host_error(format!("failed to write startup timing report: {error}"));
                        }
                    }
                    changed = true;
                } else {
                    host.syntax_worker_stopped();
                    changed = true;
                    note_ended_service(&mut ended_services, "syntax");
                }
            }
            event = receive_service_event("file picker", &ended_services, services.file_picker_events.recv()) => {
                if let Some(event) = event {
                    let paced = pace_file_picker_event(&event);
                    host.apply_event(HostEvent::FilePicker(event));
                    if paced {
                        frame_pending = true;
                    } else {
                        changed = true;
                    }
                } else {
                    note_ended_service(&mut ended_services, "file picker");
                }
            }
            event = receive_service_event("workspace search", &ended_services, services.workspace_search_events.recv()) => {
                if let Some(event) = event {
                    host.apply_event(HostEvent::WorkspaceSearch(event));
                    changed = true;
                } else {
                    note_ended_service(&mut ended_services, "workspace search");
                }
            }
            event = receive_service_event("file monitor", &ended_services, services.file_monitor_events.recv()) => {
                if let Some(event) = event {
                    host.apply_event(HostEvent::FileObservation(event));
                    changed = true;
                } else {
                    note_ended_service(&mut ended_services, "file monitor");
                }
            }
            event = receive_service_event("Git monitor", &ended_services, services.git_monitor_events.recv()) => {
                if let Some(event) = event {
                    host.apply_event(HostEvent::GitInvalidation(event));
                    changed = true;
                    changed |= host.refresh_git_if_due(Instant::now());
                } else {
                    note_ended_service(&mut ended_services, "Git monitor");
                }
            }
            output = services.terminal_events.recv() => {
                if let Some(output) = output {
                    let observed = active.is_some();
                    // Only sizes are recorded: the bytes are the child's
                    // output and may be as private as anything typed.
                    #[cfg(debug_assertions)]
                    let mut received = vec![terminal_output_summary(&output)];
                    host.apply_terminal_output(output, observed);
                    terminal::drain(&mut services.terminal_events, |output| {
                        #[cfg(debug_assertions)]
                        received.push(terminal_output_summary(&output));
                        host.apply_terminal_output(output, observed);
                    });
                    #[cfg(debug_assertions)]
                    trace_host_event(
                        input_trace.as_mut(),
                        format_args!("output {} observed={observed}", received.join(" ")),
                    )?;
                    frame_pending = true;
                }
            }
            event = receive_optional_service_event(&mut services.workspace_events) => {
                if let Some(event) = event {
                    let observation = matches!(&event, HostEvent::Workspace(runyte::workspace::WorkspaceEvent::Observed { .. }));
                    let before = observation.then(|| (host.app().session_strip_snapshot(), host.app().status.clone(), host.app().status_error,
                        host.app().workspace_number, host.app().overlay_snapshots()));
                    host.apply_event(event);
                    changed = !observation || before != Some((host.app().session_strip_snapshot(), host.app().status.clone(), host.app().status_error,
                        host.app().workspace_number, host.app().overlay_snapshots()));
                }
            }
            event = async {
                #[cfg(windows)]
                { match services.native_catalog_events.as_mut() {
                    Some(events) => events.recv().await,
                    None => std::future::pending().await,
                }}
                #[cfg(not(windows))]
                { std::future::pending::<Option<()>>().await }
            } => {
                #[cfg(windows)]
                if let Some(event) = event {
                    host.apply_event(HostEvent::Workspace(event));
                    changed = true;
                } else {
                    services.native_catalog_events = None;
                    host.app_mut().detach_workspace_service();
                    note_ended_service(&mut ended_services, "native session catalog");
                }
                #[cfg(not(windows))]
                let _ = event;
            }
            event = receive_optional_service_event(&mut services.git_events) => {
                if let Some(event) = event {
                    host.apply_event(HostEvent::Git(event));
                    changed = true;
                }
            }
            _ = refresh_tick.tick() => {
                report_logging_failure(host.app_mut());
                services.file_monitor.sync(host.file_monitor_requests());
                services.git_monitor.sync(host.git_monitor_repository());
                changed = host.refresh_git_if_due(Instant::now());
                changed |= host.app_mut().poll_external_opens(Instant::now());
                if active.is_some() { changed |= host.refresh_session_activity(); }
            }
            _ = idle_tick.tick() => {
                if let Some(supervisor) = supervising_parent.as_ref()
                    && supervisor.exited()?
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
            _ = finder_refresh_tick.tick(), if host.finder_terminals_dirty() => {
                // A content refresh drops the rows it is about to read back,
                // so this state is a hole rather than an answer. The pass that
                // refills it decides when there is a frame worth publishing.
                if host.refresh_finder_terminals() && !host.resource_finder_scan_pending() {
                    changed = true;
                }
            }
            _ = tokio::task::yield_now(), if host.resource_finder_scan_pending() => {
                host.advance_resource_finder_scan();
                // A slice is one of many states a pass moves through; only the
                // one that ends it is worth publishing on its own. The rest
                // wait for the frame tick, which holds them back entirely
                // while a refresh is refilling.
                if host.resource_finder_scan_pending() {
                    frame_pending = true;
                } else {
                    changed = true;
                }
            }
            // Nothing a refill passes through is worth publishing: between
            // dropping a terminal's rows and finding them again the list has a
            // hole where results the reader was looking at used to be.
            _ = frame_tick.tick(), if (frame_pending || host.app().diff_work_pending())
                && active.is_some()
                && !host.finder_scan_refills() =>
            {
                changed = frame_pending || host.app_mut().poll_diff_work();
            }
            _ = tokio::time::sleep(pointer_autoscroll.unwrap_or_default()), if pointer_autoscroll.is_some() && active.is_some() => {
                changed = host.advance_pointer_autoscroll(Instant::now());
            }
            // Paced picker state comes due without an event to carry it: a
            // ranked answer waiting for the rows to age out, and header
            // counts the work stopped short of. Only an attached client can
            // be shown either, so a detached host has nothing to wake for —
            // and, like a frame, neither is published through a refill.
            _ = tokio::time::sleep(picker_pacing.unwrap_or_default()),
                if picker_pacing.is_some() && active.is_some() && !host.finder_scan_refills() =>
            {
                host.advance_picker_pacing();
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
        let expired = parent_switches
            .iter()
            .filter(|(_, (_, started))| started.elapsed() >= Duration::from_secs(30))
            .map(|(receipt, _)| receipt.clone())
            .collect::<Vec<_>>();
        for receipt in expired {
            if let Some((child, _)) = parent_switches.remove(&receipt) {
                send_control_response(&mut controls, child, HostResponse::Error { message: "parent attachment did not finish within 30 seconds; inspect the outer TUI before retrying".to_owned() });
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
        if frame_publication_ready(changed, host.finder_scan_refills(), &mut frame_pending) {
            #[cfg_attr(not(debug_assertions), allow(unused_variables))]
            let published = publish_attached_frame(&mut host, &mut active, &key_hints);
            #[cfg(debug_assertions)]
            trace_host_event(
                input_trace.as_mut(),
                format_args!("published {published:?}"),
            )?;
            frame_pending = false;
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
    host.shutdown_plugins().await?;
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

/// What one publication put in the attached client's visual slot, for the
/// development input trace: the response kind and the frame it carries.
#[cfg(unix)]
type FramePublication = Option<(&'static str, u64)>;

#[cfg(unix)]
fn publish_attached_frame(
    host: &mut WorkspaceHost,
    active: &mut Option<AttachedClient>,
    key_hints: &KeyHintState,
) -> FramePublication {
    let client = active.as_mut()?;
    host.mark_visible_terminals_viewed();
    let frame: runyte::protocol::HostFrame = host
        .prepare_frame_with_hints(client.geometry, Some(key_hints))
        .into();
    for request in host.app_mut().media_requests.drain(..) {
        if client
            .responses
            .try_send(HostResponse::MediaAction {
                frame: frame.id,
                pane: request.pane,
                path: encode_path(&request.path),
                page: request.page,
                action: request.action.into(),
            })
            .is_err()
        {
            *active = None;
            return None;
        }
    }
    let response = if client.responses.visual_pending() {
        // Replacing an unseen delta with another delta would make the latter's
        // base impossible for the client to have. A complete replacement is
        // still one bounded slot and lets the client converge without a
        // resynchronization loop under continuous output.
        HostResponse::Frame {
            frame: Box::new(frame.clone()),
        }
    } else {
        match client.last_frame.as_ref() {
            Some(base) => {
                if let Some(damage) = runyte::protocol::EditorDamageFrame::between(base, &frame) {
                    HostResponse::EditorDamage {
                        damage: Box::new(damage),
                    }
                } else if let Some(damage) =
                    runyte::protocol::TerminalDamageFrame::between(base, &frame)
                {
                    HostResponse::TerminalDamage {
                        damage: Box::new(damage),
                    }
                } else {
                    HostResponse::Frame {
                        frame: Box::new(frame.clone()),
                    }
                }
            }
            None => HostResponse::Frame {
                frame: Box::new(frame.clone()),
            },
        }
    };
    // The replaceable slot carries a complete snapshot whenever its prior
    // visual is still pending, so a slow client can skip to the newest state.
    // Only a closed connection means the client is actually gone. Detaching on a merely
    // full channel used to end the session mid-keystroke, which reached the
    // person as an unexplained clean exit.
    let kind = match &response {
        HostResponse::Frame { .. } => "frame",
        HostResponse::EditorDamage { .. } => "editor-damage",
        HostResponse::TerminalDamage { .. } => "terminal-damage",
        _ => "other",
    };
    let id = frame.id.get();
    match client.responses.try_send(response) {
        Ok(()) => {
            client.last_frame = Some(frame);
            return Some((kind, id));
        }
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
    None
}

#[cfg(all(unix, debug_assertions))]
fn terminal_output_summary(output: &terminal::TerminalOutput) -> String {
    match output {
        terminal::TerminalOutput::Bytes { id, bytes } => format!("{id}:{}b", bytes.len()),
        terminal::TerminalOutput::Exited { id, code } => format!("{id}:exit={code:?}"),
    }
}

/// Adds one host event other than input to the development input trace, so
/// a keystroke can be ordered against the output and frames that followed it.
#[cfg(all(unix, debug_assertions))]
fn trace_host_event(trace: Option<&mut impl Write>, event: std::fmt::Arguments<'_>) -> Result<()> {
    let Some(trace) = trace else {
        return Ok(());
    };
    writeln!(trace, "host {event}").context("failed to write RUNYTE_INPUT_TRACE")?;
    trace.flush().context("failed to flush RUNYTE_INPUT_TRACE")
}

fn dispatch_host_key_or_text(
    host: &mut WorkspaceHost,
    key_hints: &mut KeyHintState,
    input: InputEvent,
    repeated: bool,
) -> HintEventResult {
    let hint_result = observe_key_or_text_hint(host.app(), key_hints, &input);
    if hint_result != HintEventResult::Forward {
        return hint_result;
    }
    let dispatches = motion_repeat_dispatches(host.app(), &input, repeated);
    for _ in 0..dispatches {
        let result = host.execute_frontend_input(input.clone(), repeated);
        if let Err(error) = result {
            host.report_host_error(error.to_string());
            break;
        }
    }
    hint_result
}

#[cfg(windows)]
fn dispatch_host_repeated_key_or_text(
    host: &mut WorkspaceHost,
    key_hints: &mut KeyHintState,
    input: InputEvent,
) {
    let hint_result = observe_key_or_text_hint(host.app(), key_hints, &input);
    if hint_result != HintEventResult::Forward {
        return;
    }
    let dispatches = motion_repeat_dispatches(host.app(), &input, true);
    for _ in 0..dispatches {
        if let Err(error) = host.execute_frontend_input(input.clone(), true) {
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
        InputEvent::Key(_) | InputEvent::Text(_) | InputEvent::ClipboardPaste => {
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
    let mut options = fs::OpenOptions::new();
    options.create(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NONBLOCK);
    }
    let file = options.open(&path).with_context(|| {
        format!(
            "failed to open RUNYTE_INPUT_TRACE path {}",
            Path::new(&path).display()
        )
    })?;
    anyhow::ensure!(
        file.metadata()?.is_file(),
        "RUNYTE_INPUT_TRACE requires a regular file"
    );
    file.set_len(0)
        .context("failed to truncate RUNYTE_INPUT_TRACE")?;
    Ok(Some(file))
}

#[cfg(debug_assertions)]
fn trace_input(
    trace: Option<&mut impl Write>,
    phase: &str,
    app: &App,
    input: &InputEvent,
    sensitive: bool,
    repeated: bool,
    hint: Option<HintEventResult>,
) -> Result<()> {
    let Some(trace) = trace else {
        return Ok(());
    };
    let input = if sensitive {
        "<application input redacted>".to_owned()
    } else {
        format!("{input:?}")
    };
    let terminal = app.active_terminal();
    let reviewing = terminal
        .and_then(|id| app.terminals.get(id))
        .is_some_and(|session| session.reviewing());
    writeln!(
        trace,
        "{phase} input={input} repeated={repeated} hint={hint:?} mode={:?} pane={} \
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
fn unix_native_switch_refusal(request: &ClientRequest) -> Option<HostResponse> {
    matches!(
        request,
        ClientRequest::NativeSwitchCommit { .. }
            | ClientRequest::NativeParentSwitchCommitObserved { .. }
            | ClientRequest::NativeSwitchAbort { .. }
    )
    .then(|| HostResponse::Refused {
        message: "provisional native session switching is unavailable on this host".to_owned(),
    })
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
            target: Box::new(match request.target {
                runyte::app::WorkspaceSwitchTarget::UserSelector(selector) => {
                    runyte::protocol::WorkspaceSwitchTarget::UserSelector {
                        selector_bytes: encode_path(&selector),
                    }
                }
                runyte::app::WorkspaceSwitchTarget::Selected(selection) => {
                    runyte::protocol::WorkspaceSwitchTarget::Selected {
                        project_root_bytes: encode_path(selection.project_root()),
                        publication_key: selection
                            .publication_key()
                            .map(runyte::workspace::PublicationKey::to_bytes),
                    }
                }
                runyte::app::WorkspaceSwitchTarget::Previous => {
                    runyte::protocol::WorkspaceSwitchTarget::Previous
                }
            }),
            working_directory_bytes: encode_path(&request.working_directory),
            running_only: request.running_only,
            visit: request
                .visit
                .map(|visit| runyte::protocol::DestinationVisit {
                    incarnation: visit.incarnation,
                    destination: match visit.destination {
                        runyte::app::OpenDestination::Buffer(index) => {
                            runyte::protocol::OpenDestination::Buffer(index as u64 + 1)
                        }
                        runyte::app::OpenDestination::Terminal(id) => {
                            runyte::protocol::OpenDestination::Terminal(id.get())
                        }
                    },
                }),
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

fn terminal_color_depth() -> ui::TerminalColorDepth {
    ui::TerminalColorDepth::from_color_count(crossterm::style::available_color_count())
}

#[cfg(unix)]
trait AttachedSurface {
    fn native(&self) -> bool {
        false
    }
    fn resize_surface(&mut self, area: ratatui::layout::Rect) -> io::Result<()>;
    fn draw_host(
        &mut self,
        snapshot: &runyte::workspace::HostFrame,
        depth: ui::TerminalColorDepth,
    ) -> io::Result<()>;
}
#[cfg(unix)]
impl AttachedSurface for Terminal<CrosstermBackend<std::io::Stdout>> {
    fn resize_surface(&mut self, area: ratatui::layout::Rect) -> io::Result<()> {
        self.resize(area)
    }
    fn draw_host(
        &mut self,
        snapshot: &runyte::workspace::HostFrame,
        depth: ui::TerminalColorDepth,
    ) -> io::Result<()> {
        self.draw(|frame| ui::render_host_frame(frame, snapshot, depth))?;
        Ok(())
    }
}
#[cfg(all(unix, feature = "native"))]
impl AttachedSurface for native_frontend::Surface {
    fn native(&self) -> bool {
        matches!(self, Self::Native(_))
    }
    fn resize_surface(&mut self, area: ratatui::layout::Rect) -> io::Result<()> {
        self.resize(area)
    }
    fn draw_host(
        &mut self,
        snapshot: &runyte::workspace::HostFrame,
        depth: ui::TerminalColorDepth,
    ) -> io::Result<()> {
        self.draw_host_frame(snapshot, depth)
    }
}

/// Attaches, and keeps attaching wherever the editor asks to go next.
///
/// One process for the whole session. The previous arrangement replaced the
/// re-exec by spawning a child `runyte --mux` and blocking on it, so moving
/// from one workspace to another and back again stacked processes and quitting
/// unwound a stack.
#[cfg(unix)]
async fn run_workspace_switcher(
    endpoint: LocalEndpoint,
    mouse_enabled: bool,
    window: bool,
    cwd_file: Option<&Path>,
    config: &Config,
    config_path: Option<&Path>,
) -> Result<()> {
    let color_depth = if window {
        ui::TerminalColorDepth::TrueColor
    } else {
        terminal_color_depth()
    };
    let mut termination = TerminationSignals::new()?;
    let _terminal = if window {
        None
    } else {
        Some(TerminalGuard::enter(mouse_enabled)?)
    };
    #[cfg(feature = "native")]
    let mut terminal = native_frontend::Surface::new(CrosstermBackend::new(stdout()), window)?;
    #[cfg(not(feature = "native"))]
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout()))?;
    #[cfg(feature = "native")]
    let mut geometry = if window {
        let (width, height) = native_frontend::dimensions();
        ui::frame_geometry(ratatui::layout::Rect::new(0, 0, width, height))
    } else {
        current_frame_geometry()?
    };
    #[cfg(not(feature = "native"))]
    let mut geometry = current_frame_geometry()?;
    let mut terminal_events = AttachedTerminalEvents::for_frontend(window)?;
    let mut current = endpoint;
    let mut previous: Option<LocalEndpoint> = None;
    let mut notice: Option<String> = None;
    let mut history = AttachmentHistory::default();
    let mut parent_handoff: Option<ParentHandoff> = None;
    let mut pending_visit = None;
    let mut quit_returns: Option<std::collections::VecDeque<LocalEndpoint>> = None;
    loop {
        let attachment = tokio::select! {
            attachment = run_attached(&current, &mut terminal, &mut terminal_events, &mut geometry,
                AttachOptions { wait_token: None, cwd_file, notice: notice.take(), color_depth,
                    history: Some(&mut history), parent_handoff: Some(&mut parent_handoff), visit: pending_visit.take() }) => attachment,
            signal = termination.recv() => return Err(terminated(signal)),
        };
        if let Some(handoff) = parent_handoff.take() {
            let error = match &attachment {
                Err(error) => format!("{error:#}"),
                Ok(AttachOutcome::Refused(message)) => message.clone(),
                _ => "destination attachment ended before its first frame".to_owned(),
            };
            let _ = complete_parent_handoff(&handoff, Some(error)).await;
        }
        // The session we just closed cannot recover a refused attachment.
        // Try the remaining live destinations without restarting any host.
        if let Some(candidates) = quit_returns.as_mut()
            && matches!(&attachment, Err(_) | Ok(AttachOutcome::Refused(_)))
        {
            if let Some(target) = candidates.pop_front() {
                current = target;
                continue;
            }
            return Ok(());
        }
        quit_returns = None;
        let Some(outcome) =
            recover_switched_attachment(attachment, &mut current, &mut previous, &mut notice)?
        else {
            continue;
        };
        match outcome {
            AttachOutcome::Detached => return Ok(()),
            AttachOutcome::Quit => {
                let closed = current.project_root().to_owned();
                history.forget(&closed);
                previous = None;
                let mut candidates = quit_return_targets(
                    &closed,
                    &history,
                    known_workspaces_for_navigation(&config.workspace.state).await?,
                );
                let Some(target) = candidates.pop_front() else {
                    return Ok(());
                };
                current = target;
                quit_returns = Some(candidates);
            }
            AttachOutcome::Switch {
                target,
                working_directory,
                running_only,
                parent_receipt,
                visit,
            } => {
                if let Some(receipt) = parent_receipt {
                    parent_handoff = Some(ParentHandoff {
                        source: current.clone(),
                        receipt,
                    });
                }
                let selector = match unix_switch_selector(target, history.previous.as_ref()) {
                    Ok(Some(selector)) => selector,
                    Ok(None) => {
                        notice = Some("No previous persistent session".to_owned());
                        continue;
                    }
                    Err(error) => {
                        let prepared = Err(error);
                        apply_prepared_switch(prepared, &mut current, &mut previous, &mut notice);
                        continue;
                    }
                };
                let prepared = if running_only {
                    resolve_registered_host_from_directory(&selector, &working_directory)
                        .map(|host| (host.project_root != current.project_root()).then(|| host.endpoint().clone()))
                        .context("the destination persistent session is no longer available; it was not restarted")
                } else {
                    prepare_switch_target(
                        &selector,
                        &working_directory,
                        &current,
                        config,
                        config_path,
                    )
                    .await
                };
                if let Err(error) = &prepared
                    && let Some(handoff) = parent_handoff.take()
                {
                    let _ = complete_parent_handoff(&handoff, Some(format!("{error:#}"))).await;
                }
                if prepared.is_ok() {
                    pending_visit = visit;
                }
                apply_prepared_switch(prepared, &mut current, &mut previous, &mut notice);
            }
            AttachOutcome::Refused(message) => match previous.take() {
                Some(source) => {
                    current = source;
                    notice = Some(message);
                }
                None => anyhow::bail!(message),
            },
        }
    }
}

#[cfg(unix)]
fn unix_switch_selector(
    target: runyte::protocol::WorkspaceSwitchTarget,
    previous: Option<&LocalEndpoint>,
) -> Result<Option<PathBuf>> {
    match target {
        runyte::protocol::WorkspaceSwitchTarget::Previous => {
            Ok(previous.map(|endpoint| endpoint.project_root().to_owned()))
        }
        runyte::protocol::WorkspaceSwitchTarget::UserSelector { selector_bytes } => {
            Ok(Some(decode_path(selector_bytes)?))
        }
        runyte::protocol::WorkspaceSwitchTarget::Selected {
            publication_key: Some(_),
            ..
        } => anyhow::bail!(
            "native publication selections cannot be resolved by the Unix session switcher"
        ),
        runyte::protocol::WorkspaceSwitchTarget::Selected {
            project_root_bytes,
            publication_key: None,
        } => Ok(Some(decode_path(project_root_bytes)?)),
    }
}

#[cfg(unix)]
#[derive(Default)]
struct AttachmentHistory {
    previous: Option<LocalEndpoint>,
    recent: Vec<LocalEndpoint>,
}
#[cfg(unix)]
impl AttachmentHistory {
    fn attached(&mut self, endpoint: &LocalEndpoint) -> Option<PathBuf> {
        self.recent
            .retain(|entry| entry.project_root() != endpoint.project_root());
        self.recent.push(endpoint.clone());
        if self.recent.len() > 256 {
            self.recent.remove(0);
        }
        self.update();
        self.previous
            .as_ref()
            .map(|endpoint| endpoint.project_root().to_owned())
    }

    fn forget(&mut self, root: &Path) {
        self.recent.retain(|entry| entry.project_root() != root);
        self.update();
    }

    fn update(&mut self) {
        self.previous = self.recent.iter().rev().nth(1).cloned();
    }
}

/// Prefer this TUI's successful visits, then the catalog's recent activity.
/// Resolve only registered hosts; stopped history must never be restarted.
#[cfg(unix)]
fn quit_return_targets(
    closed: &Path,
    history: &AttachmentHistory,
    mut rows: Vec<runyte::workspace::WorkspaceRow>,
) -> std::collections::VecDeque<LocalEndpoint> {
    rows.retain(|row| {
        row.running
            && row.project_root != closed
            && row.incompatible_protocol.is_none()
            && row.interactive_attached != Some(true)
    });
    rows.sort_by_key(|row| {
        std::cmp::Reverse((
            history
                .recent
                .iter()
                .position(|entry| entry.project_root() == row.project_root),
            row.last_active_unix_seconds,
        ))
    });
    rows.into_iter()
        .filter_map(|row| {
            resolve_registered_host_from_directory(&row.project_root, closed)
                .ok()
                .map(|host| host.endpoint().clone())
        })
        .collect()
}

#[cfg(unix)]
struct ParentHandoff {
    source: LocalEndpoint,
    receipt: String,
}

#[cfg(unix)]
async fn complete_parent_handoff(handoff: &ParentHandoff, error: Option<String>) -> Result<()> {
    tokio::time::timeout(Duration::from_secs(3), async {
        let mut control = connect_control(&handoff.source).await?;
        control
            .send(&ClientRequest::ParentHandoffResult {
                receipt: handoff.receipt.clone(),
                error: error.map(|error| bounded_destination_label(&error)),
            })
            .await?;
        match control.recv().await? {
            Some(HostResponse::ParentAttached) => Ok(()),
            Some(HostResponse::Error { message }) => anyhow::bail!(message),
            _ => anyhow::bail!("owning host did not acknowledge parent handoff completion"),
        }
    })
    .await
    .context("owning host did not acknowledge parent handoff completion")?
}

#[cfg(unix)]
fn apply_prepared_switch(
    prepared: Result<Option<LocalEndpoint>>,
    current: &mut LocalEndpoint,
    previous: &mut Option<LocalEndpoint>,
    notice: &mut Option<String>,
) {
    match prepared {
        Ok(Some(next)) => {
            *previous = Some(std::mem::replace(current, next));
        }
        // Already attached here; the editor asked for the workspace it is in,
        // so there is nothing to move to.
        Ok(None) => {}
        Err(error) => *notice = Some(format!("{error:#}")),
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
        start_workspace_switch_host(&endpoint, startup).await?;
    }
    Ok(Some(endpoint))
}

#[cfg(unix)]
async fn start_workspace_switch_host(endpoint: &LocalEndpoint, startup: HostStartup) -> Result<()> {
    match start_detached_host(endpoint, startup).await {
        Err(error)
            if error
                .downcast_ref::<UnavailableStartupExecutable>()
                .is_some() =>
        {
            Err(error).context(
                "detach with :detach and launch Runyte again, then retry the workspace switch",
            )
        }
        outcome => outcome,
    }
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
    terminal_loss: &mut TerminalLoss,
    launching_parent: &HostSupervisor,
) -> Result<()> {
    let color_depth = terminal_color_depth();
    let _terminal = TerminalGuard::enter(mouse_enabled)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout()))?;
    let mut geometry = current_frame_geometry()?;
    let mut terminal_events = AttachedTerminalEvents::isolated_wait_reader()?;
    let (mut attachment, reconcile_lifecycle) = tokio::select! {
        biased;
        signal = termination.recv() => (Err(terminated(signal)), false),
        parent = launching_parent.recv() => {
            (Err(parent.map_or_else(
                |error| error.context("failed while watching the wait client's launching process"),
                |()| anyhow::anyhow!("wait request lost its launching process before completion"),
            )), false)
        }
        loss = terminal_loss.recv() => {
            let error = match loss {
                Ok(()) => terminal_loss_error(termination).await,
                Err(error) => error,
            };
            (Err(error), false)
        }
        attachment = run_attached(
            endpoint,
            &mut terminal,
            &mut terminal_events,
            &mut geometry,
            AttachOptions {
                wait_token: Some(token),
                cwd_file: None,
                notice: None,
                color_depth,
                history: None,
                parent_handoff: None,
                visit: None,
            },
        ) => (attachment, true),
    };
    if reconcile_lifecycle && let Err(error) = attachment {
        attachment = Err(prefer_wait_lifecycle_error(error, termination, terminal_loss).await);
    }
    release_wait_terminal(terminal);
    match attachment {
        Err(attachment_error)
            if attachment_error
                .downcast_ref::<TerminatedBySignal>()
                .is_some() =>
        {
            Err(attachment_error)
        }
        // Completing a wait closes its interactive attachment after queuing
        // terminal state. Transport failure or terminal loss can race that
        // close, so the independent control connection resolves authoritative
        // durable status before the client reports failure.
        Err(attachment_error) => {
            recover_wait_after_lifecycle_loss(control, token, false, attachment_error).await
        }
        Ok(AttachOutcome::Detached | AttachOutcome::Quit) => Ok(()),
        Ok(AttachOutcome::Switch { .. }) => {
            anyhow::bail!("wait request cannot switch workspaces")
        }
        Ok(AttachOutcome::Refused(message)) => anyhow::bail!(message),
    }
}

/// Terminal input used by a persistent attachment.
///
/// Crossterm 0.29 can remain inside its Unix event reader forever after a PTY
/// hangup. Its `EventStream::poll_next` then blocks the Tokio thread on the
/// process-global reader mutex, preventing `attach_for_wait` from observing
/// the independent terminal-loss watcher. A `--wait` process therefore reads
/// input on a detached OS thread and receives events through a channel. The
/// reader may remain blocked after a dead terminal, but it cannot block the
/// lifecycle executor, and the dedicated wait process exits immediately after
/// releasing its request.
#[cfg(unix)]
struct AttachedTerminalEvents {
    source: AttachedEventSource,
    /// The opt-in development trace of what this client sent to its host. It
    /// lives here because this value, unlike any one attachment, spans every
    /// workspace the client visits.
    #[cfg(debug_assertions)]
    trace: Option<fs::File>,
}

#[cfg(unix)]
enum AttachedEventSource {
    #[cfg(feature = "native")]
    Native(native_frontend::Events),
    Stream(EventStream),
    Isolated(tokio::sync::mpsc::UnboundedReceiver<io::Result<CrosstermEvent>>),
}

#[cfg(unix)]
impl AttachedTerminalEvents {
    fn new(source: AttachedEventSource) -> Result<Self> {
        Ok(Self {
            source,
            #[cfg(debug_assertions)]
            trace: open_input_trace()?,
        })
    }

    fn for_frontend(window: bool) -> Result<Self> {
        #[cfg(feature = "native")]
        if window {
            return Self::new(AttachedEventSource::Native(native_frontend::Events::new(
                true,
            )));
        }
        let _ = window;
        Self::stream()
    }

    fn presented(&self, current: runyte::workspace::FrameId) -> Option<runyte::protocol::FrameId> {
        #[cfg(feature = "native")]
        if let AttachedEventSource::Native(events) = &self.source {
            return events.presented_frame(None).map(Into::into);
        }
        Some(current.into())
    }

    fn stream() -> Result<Self> {
        Self::new(AttachedEventSource::Stream(EventStream::new()))
    }

    /// Records one input as it leaves for the host.
    ///
    /// Whether a character key belongs to a plugin's private input is the
    /// host's knowledge, not the client's, so every character is redacted
    /// here. What the host did with it is in the host's own trace.
    #[cfg(debug_assertions)]
    fn trace_sent(
        &mut self,
        input: &InputEvent,
        repeated: bool,
        presented: runyte::protocol::FrameId,
    ) -> Result<()> {
        let Some(trace) = self.trace.as_mut() else {
            return Ok(());
        };
        let input = match input {
            InputEvent::Key(KeyStroke {
                code: runyte::input::KeyCode::Char(_),
                modifiers,
            }) => format!("Key(<character> {modifiers:?})"),
            input => format!("{input:?}"),
        };
        writeln!(
            trace,
            "client sent input={input} repeated={repeated} presented={}",
            presented.get()
        )
        .context("failed to write RUNYTE_INPUT_TRACE")?;
        trace.flush().context("failed to flush RUNYTE_INPUT_TRACE")
    }

    /// Records one visual response and whether it reached the screen, so the
    /// trace can say whether output the host published was ever drawn.
    #[cfg(debug_assertions)]
    fn trace_received(&mut self, kind: &str, id: u64, applied: bool) -> Result<()> {
        let Some(trace) = self.trace.as_mut() else {
            return Ok(());
        };
        writeln!(trace, "client received {kind} frame={id} applied={applied}")
            .context("failed to write RUNYTE_INPUT_TRACE")?;
        trace.flush().context("failed to flush RUNYTE_INPUT_TRACE")
    }

    fn isolated_wait_reader() -> Result<Self> {
        let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
        thread::Builder::new()
            .name("runyte-wait-terminal-input".into())
            .spawn(move || {
                loop {
                    let event = crossterm::event::read();
                    let terminal = event.is_err();
                    if sender.send(event).is_err() || terminal {
                        break;
                    }
                }
            })
            .context("failed to start wait terminal input reader")?;
        Self::new(AttachedEventSource::Isolated(receiver))
    }

    async fn next(&mut self) -> Option<io::Result<CrosstermEvent>> {
        match &mut self.source {
            #[cfg(feature = "native")]
            AttachedEventSource::Native(events) => events.next().await,
            AttachedEventSource::Stream(stream) => stream.next().await,
            AttachedEventSource::Isolated(receiver) => receiver.recv().await,
        }
    }
}

/// Prevents Ratatui's destructor from reporting a failed cursor restore to a
/// stderr that disappeared with the same PTY.
///
/// A reachable terminal accepts the explicit cursor restore and Ratatui then
/// has no destructor work left. An unreachable one cannot be restored; leaking
/// this small process-local renderer avoids Ratatui retrying the write through
/// `eprintln!`, whose own failure would panic and replace the lifecycle status
/// with exit code 101. The wait client exits immediately afterward.
#[cfg(unix)]
fn release_wait_terminal(mut terminal: Terminal<CrosstermBackend<std::io::Stdout>>) {
    if terminal.show_cursor().is_err() {
        std::mem::forget(terminal);
    }
}

/// Lets terminal lifecycle evidence outrank a rendering or transport failure
/// that became observable at the same instant.
///
/// Closing a PTY can make a frame write fail before the exceptional-condition
/// watcher or SIGHUP handler is scheduled. Without this bounded reconciliation
/// window that ordinary I/O error bypasses the terminal-loss status and signal
/// semantics even though all three events have the same cause.
#[cfg(unix)]
async fn prefer_wait_lifecycle_error(
    attachment_error: anyhow::Error,
    termination: &mut TerminationSignals,
    terminal_loss: &mut TerminalLoss,
) -> anyhow::Error {
    tokio::select! {
        biased;
        signal = termination.recv() => terminated(signal),
        loss = terminal_loss.recv() => match loss {
            Ok(()) => terminal_loss_error(termination).await,
            Err(error) => error,
        },
        _ = tokio::time::sleep(Duration::from_millis(50)) => attachment_error,
    }
}

/// How one attachment ended, so the switcher can decide what to do next.
#[cfg(unix)]
enum AttachOutcome {
    /// The person is finished with this client.
    Detached,
    /// The host stopped; continue in another running persistent session.
    Quit,
    /// The editor asked to move to another workspace.
    Switch {
        target: runyte::protocol::WorkspaceSwitchTarget,
        working_directory: std::path::PathBuf,
        running_only: bool,
        parent_receipt: Option<String>,
        visit: Option<runyte::protocol::DestinationVisit>,
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

/// Client-local state that changes how one host attachment is presented.
#[cfg(unix)]
struct AttachOptions<'a> {
    wait_token: Option<WaitToken>,
    cwd_file: Option<&'a Path>,
    notice: Option<String>,
    color_depth: ui::TerminalColorDepth,
    history: Option<&'a mut AttachmentHistory>,
    parent_handoff: Option<&'a mut Option<ParentHandoff>>,
    visit: Option<runyte::protocol::DestinationVisit>,
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
    terminal: &mut impl AttachedSurface,
    terminal_events: &mut AttachedTerminalEvents,
    geometry: &mut runyte::app::FrameGeometry,
    options: AttachOptions<'_>,
) -> Result<AttachOutcome> {
    let AttachOptions {
        wait_token,
        cwd_file,
        notice,
        color_depth,
        history,
        parent_handoff,
        visit,
    } = options;
    #[cfg(feature = "native")]
    if terminal.native() {
        native_frontend::begin_attachment();
    }
    let mut client = BufferedLocalClient::connect_with_media(
        endpoint,
        *geometry,
        cwd_file.is_some(),
        terminal.native(),
    )
    .await?;
    match client.recv_handshake().await? {
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
    terminal.resize_surface(ratatui::layout::Rect::new(
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
    if let Some(visit) = visit {
        client
            .send(&ClientRequest::VisitDestination {
                incarnation: visit.incarnation,
                destination: visit.destination,
            })
            .await?;
    }
    if let Some(history) = history {
        history.attached(endpoint);
    }
    if let Some(parent_handoff) = parent_handoff
        && let Some(handoff) = parent_handoff.take()
        && let Err(error) = complete_parent_handoff(&handoff, None).await
    {
        client
            .send(&ClientRequest::Notify {
                message: format!(
                    "Attached, but the originating shell could not be notified: {error:#}"
                ),
            })
            .await?;
    }
    if let Some(token) = wait_token {
        client.send(&ClientRequest::AttachWait { token }).await?;
        loop {
            match client.recv().await? {
                Some(HostResponse::Frame { frame }) => {
                    current_frame = (*frame)
                        .try_into()
                        .map_err(|error: String| anyhow::anyhow!(error))?;
                    terminal.draw_host(&current_frame, color_depth)?;
                }
                Some(HostResponse::TerminalDamage { damage }) => {
                    if apply_terminal_damage(&mut current_frame, &damage)? {
                        terminal.draw_host(&current_frame, color_depth)?;
                    } else {
                        client.send(&ClientRequest::Resynchronize).await?;
                    }
                }
                Some(HostResponse::EditorDamage { damage }) => {
                    if apply_editor_damage(&mut current_frame, &damage)? {
                        terminal.draw_host(&current_frame, color_depth)?;
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
    terminal.draw_host(&current_frame, color_depth)?;
    let mut key_repeat_detector = KeyRepeatDetector::default();
    let mut pointer_batcher = PointerBatcher::default();
    let mut pointer_tick = tokio::time::interval(Duration::from_millis(8));
    pointer_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut wait_tick = tokio::time::interval(Duration::from_millis(100));
    wait_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            input = terminal_events.next() => {
                #[cfg(feature = "native")]
                if terminal.native() {
                    if native_frontend::take_close_request() {
                        client.send(&ClientRequest::Detach).await?;
                        continue;
                    }
                    for request in native_frontend::attached_media_requests(&current_frame) { client.send(&request).await?; }
                    if let AttachedEventSource::Native(events) = &terminal_events.source
                        && events.is_presentation_acknowledgement() {
                        if let Some(frame) = terminal_events.presented(current_frame.id) {
                            client.send(&ClientRequest::FrameDrawn { frame }).await?;
                        }
                        continue;
                    }
                }
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
                let presented = terminal_events.presented(current_frame.id);
                if matches!(input, InputEvent::Pointer(_)) && presented.is_none() { continue; }
                match input {
                    InputEvent::Pointer(event) if is_wheel_event(event.kind) => {
                        if let Some(batch) = pointer_batcher.push_wheel(event, presented.unwrap().into()) {
                            client.send(&batch.request()).await?;
                        }
                    }
                    InputEvent::Pointer(event) => {
                        if let Some(batch) = pointer_batcher.take() {
                            client.send(&batch.request()).await?;
                        }
                        client.send(&ClientRequest::Pointer {
                            event: event.into(),
                            frame: presented.unwrap(),
                            repetitions: 1,
                        }).await?;
                    }
                    event => {
                        if let Some(batch) = pointer_batcher.take() {
                            client.send(&batch.request()).await?;
                        }
                        #[cfg(debug_assertions)]
                        terminal_events.trace_sent(&event, repeated, current_frame.id.into())?;
                        client
                            .send(&ClientRequest::Input {
                                event: event.into(),
                                repeated,
                                presented_frame: terminal_events.presented(current_frame.id),
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
                    #[cfg(feature = "native")]
                    Some(HostResponse::MediaAction { frame, pane, path, page, action }) if terminal.native() => {
                        native_frontend::receive_media_action(frame.into(), runyte::media::ViewRequest { pane, path: decode_path(path)?, page, action: action.into() });
                    }
                    Some(HostResponse::Frame { frame }) => {
                        current_frame = (*frame)
                            .try_into()
                            .map_err(|error: String| anyhow::anyhow!(error))?;
                        #[cfg(debug_assertions)]
                        terminal_events.trace_received("frame", current_frame.id.get(), true)?;
                        terminal.draw_host(&current_frame, color_depth)?;
                    }
                    Some(HostResponse::TerminalDamage { damage }) => {
                        let applied = apply_terminal_damage(&mut current_frame, &damage)?;
                        #[cfg(debug_assertions)]
                        terminal_events.trace_received("terminal-damage", damage.id.get(), applied)?;
                        if applied {
                            terminal.draw_host(&current_frame, color_depth)?;
                        } else {
                            client.send(&ClientRequest::Resynchronize).await?;
                        }
                    }
                    Some(HostResponse::EditorDamage { damage }) => {
                        let applied = apply_editor_damage(&mut current_frame, &damage)?;
                        #[cfg(debug_assertions)]
                        terminal_events.trace_received("editor-damage", damage.id.get(), applied)?;
                        if applied {
                            terminal.draw_host(&current_frame, color_depth)?;
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
                            (cwd_file, directory_bytes.map(decode_path).transpose()?)
                        {
                            write_cwd_file(cwd_file, &directory)?;
                        }
                        break;
                    }
                    Some(HostResponse::ShuttingDown) => {
                        anyhow::ensure!(wait_token.is_none(), "wait request ended before completion");
                        return Ok(AttachOutcome::Quit);
                    }
                    None => {
                        anyhow::bail!("workspace host disconnected without ending the attachment");
                    }
                    Some(HostResponse::SwitchWorkspace {
                        target,
                        working_directory_bytes,
                        running_only,
                        visit,
                    }) => {
                        anyhow::ensure!(
                            wait_token.is_none(),
                            "wait request was cancelled by a workspace switch"
                        );
                        return Ok(AttachOutcome::Switch {
                            target: *target,
                            working_directory: decode_path(working_directory_bytes)?,
                            running_only,
                            parent_receipt: None,
                            visit,
                        });
                    }
                    Some(HostResponse::Refused { message } | HostResponse::Error { message }) => {
                        anyhow::bail!(message);
                    }
                    Some(HostResponse::ParentSwitchWorkspace { selector, directory, receipt }) => {
                        anyhow::ensure!(wait_token.is_none(), "a wait-owned attachment cannot switch persistent sessions");
                        return Ok(AttachOutcome::Switch {
                            target: runyte::protocol::WorkspaceSwitchTarget::UserSelector {
                                selector_bytes: selector,
                            },
                            working_directory: decode_path(directory)?, running_only: false,
                            parent_receipt: Some(receipt), visit: None });
                    }
                    Some(HostResponse::Welcome { .. }) => {}
                    Some(_) => {}
                }
            }
            _ = wait_tick.tick(), if wait_token.is_some() => {
                let token = wait_token.expect("guarded by is_some");
                if let Err(error) = client.send(&ClientRequest::WaitStatus { token }).await {
                    recover_attached_wait_after_status_write(&mut client, token, error).await?;
                    break;
                }
            }
        }
    }
    Ok(AttachOutcome::Detached)
}

/// Reads a durable completion that can already be queued when the final
/// attached status poll loses a race with host shutdown.
///
/// The host sends semantic lifecycle replies before closing its write side,
/// but it can close its read side first. A simultaneously ready status tick
/// then observes `EPIPE` even though `WaitState::Completed` is already in this
/// socket's receive queue. Visual responses and an older pending status may
/// precede that completion, so drain only those and require the authoritative
/// terminal state before treating the failed write as success.
#[cfg(unix)]
async fn recover_attached_wait_after_status_write(
    client: &mut BufferedLocalClient,
    token: WaitToken,
    write_error: anyhow::Error,
) -> Result<()> {
    let mut write_error = Some(write_error);
    let recovery = tokio::time::timeout(SHUTDOWN_FLUSH_BUDGET, async {
        loop {
            match client.recv().await {
                Ok(Some(HostResponse::Frame { .. } | HostResponse::TerminalDamage { .. } | HostResponse::EditorDamage { .. })) => {}
                Ok(Some(HostResponse::WaitState {
                    token: response_token,
                    status: WaitStatus::Pending { .. },
                    ..
                })) if response_token == token => {}
                Ok(Some(HostResponse::WaitState {
                    token: response_token,
                    status: WaitStatus::Completed,
                    ..
                })) if response_token == token => return Ok(()),
                Ok(Some(HostResponse::WaitState {
                    token: response_token,
                    status: WaitStatus::Cancelled { reason },
                    ..
                })) if response_token == token => anyhow::bail!(reason),
                Ok(Some(response)) => {
                    return Err(write_error.take().unwrap().context(format!(
                        "wait status write failed before completion; next host response was {response:?}"
                    )));
                }
                Ok(None) => {
                    return Err(write_error.take().unwrap().context(
                        "wait status write failed and the host closed without a completion response",
                    ));
                }
                Err(read_error) => {
                    return Err(write_error.take().unwrap().context(format!(
                        "wait status write failed and completion could not be read: {read_error:#}"
                    )));
                }
            }
        }
    })
    .await;
    match recovery {
        Ok(result) => result,
        Err(error) => Err(write_error.take().unwrap().context(format!(
            "wait status write failed and completion did not arrive before the host flush deadline: {error}"
        ))),
    }
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
fn apply_editor_damage(
    current: &mut runyte::workspace::HostFrame,
    damage: &runyte::protocol::EditorDamageFrame,
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

/// What a `-a` from inside an integrated terminal asks the outer TUI to
/// attach to. A named selector goes as typed, and may name a directory that
/// becomes a workspace. Without one it is the shell directory's own
/// workspace: sending the bare directory would let the outer TUI make it a
/// workspace, which a bare `-a` never does.
#[cfg(unix)]
fn parent_attach_selector(
    selector: Option<&Path>,
    directory: &Path,
    configured_state: &Path,
) -> Result<PathBuf> {
    match selector {
        Some(selector) => Ok(selector.to_path_buf()),
        None => project_root::discover(directory, configured_state)?
            .context(project_root::NO_WORKSPACE_HERE),
    }
}

#[cfg(unix)]
async fn run_parent_request(
    arguments: &LaunchArguments,
    context: runyte::workspace::parent::ParentContext,
    launching_parent: Option<&HostSupervisor>,
) -> Result<()> {
    let endpoint = LocalEndpoint::from_parent_metadata(&decode_path(context.metadata)?).context(
        "Runyte parent context is stale; return to its persistent session or open a fresh terminal",
    )?;
    let directory = std::env::current_dir()?;
    let mut control = tokio::time::timeout(Duration::from_secs(3), connect_control(&endpoint))
        .await
        .context("owning Runyte host did not answer")??;
    if arguments.mode == LaunchMode::Persistent {
        let state = Config::load(arguments.config.as_deref())?.0.workspace.state;
        let selector =
            parent_attach_selector(arguments.workspace_selector.as_deref(), &directory, &state)?;
        let selector = selector.as_path();
        control
            .send(&ClientRequest::ParentAttach {
                terminal: context.terminal,
                capability: context.capability,
                selector: encode_path(selector),
                directory: encode_path(&directory),
            })
            .await?;
        match tokio::time::timeout(Duration::from_secs(32), control.recv())
            .await
            .context(
                "parent attachment did not complete; inspect the outer TUI before retrying",
            )?? {
            Some(HostResponse::ParentAttached) => return Ok(()),
            Some(HostResponse::Error { message } | HostResponse::Refused { message }) => {
                anyhow::bail!(message)
            }
            _ => anyhow::bail!(
                "owning host lost the attachment handoff; no nested editor was started"
            ),
        }
    }
    let mut termination = TerminationSignals::new()?;
    let mut terminal_loss = TerminalLoss::new()?;
    let paths = arguments
        .targets
        .iter()
        .map(|target| {
            if target.path.is_absolute() {
                target.path.clone()
            } else {
                directory.join(&target.path)
            }
        })
        .map(|path| encode_path(&path))
        .collect();
    control
        .send(&ClientRequest::ParentWait {
            terminal: context.terminal,
            capability: context.capability,
            paths,
        })
        .await?;
    let token = match tokio::time::timeout(Duration::from_secs(3), control.recv())
        .await
        .context("parent editor did not accept the request")??
    {
        Some(HostResponse::WaitCreated { token, .. }) => token,
        Some(HostResponse::Error { message } | HostResponse::Refused { message }) => {
            anyhow::bail!(message)
        }
        _ => anyhow::bail!("owning host did not create an external editor request"),
    };
    let outcome = wait_for_completion(
        &mut control,
        &endpoint,
        false,
        token,
        &mut termination,
        &mut terminal_loss,
        launching_parent.context("parent wait launch has no lifecycle watcher")?,
        true,
    )
    .await;
    if outcome.is_err() {
        let _ = control.send(&ClientRequest::CancelWait { token }).await;
    }
    outcome
}

#[cfg(unix)]
async fn run_wait(
    endpoint: LocalEndpoint,
    targets: Vec<LaunchTarget>,
    config_path: Option<std::path::PathBuf>,
    mouse_enabled: bool,
    verbosity: u8,
    log: Option<&Path>,
    launching_parent: &HostSupervisor,
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
    // Do not introduce another thread before detached host startup forks and
    // execs. A terminal lost during startup is still reported immediately by
    // the exceptional descriptor state once this watcher begins, before the
    // durable request can settle into its wait loop.
    let mut terminal_loss = TerminalLoss::new()?;
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
            &mut terminal_loss,
            launching_parent,
            false,
        )
        .await
    } else {
        attach_for_wait(
            &endpoint,
            mouse_enabled,
            token,
            &mut control,
            &mut termination,
            &mut terminal_loss,
            launching_parent,
        )
        .await
    };
    if outcome.is_err() {
        let _ = control.send(&ClientRequest::CancelWait { token }).await;
    }
    outcome
}

#[cfg(windows)]
async fn run_native_wait(
    arguments: &LaunchArguments,
    startup_trace: &mut StartupTrace,
    termination: &mut TerminationSignals,
) -> Result<()> {
    use runyte::{
        protocol::{ClientRequest, HostResponse, encode_path},
        workspace::windows_lifecycle::connect_control,
    };

    let parent = ForegroundParentSupervisor::capture()?;
    parent.ensure_alive()?;
    let (config, config_path) = Config::load(arguments.config.as_deref())?;
    startup_trace.mark(StartupPhase::ConfigLoaded);
    let directory = std::env::current_dir()?;
    let requested = match arguments.project_root.as_deref() {
        Some(root) => resolve_requested_project_root(&directory, root)?,
        None => match project_root::discover(&directory, &config.workspace.state)? {
            Some(root) => root,
            None => resolve_requested_project_root(&directory, &directory)?,
        },
    };
    let paths = arguments
        .targets
        .iter()
        .map(|target| {
            if target.path.is_absolute() {
                target.path.clone()
            } else {
                directory.join(&target.path)
            }
        })
        .collect::<Vec<_>>();
    let roots = CapturedRoots::capture();
    let mut reserved_user_roots = config_path
        .as_deref()
        .map(|path| config::config_root_for(path, &directory))
        .into_iter()
        .collect::<Vec<_>>();
    if let Some(cache) = external_open::cache_root() {
        reserved_user_roots.push(cache);
    }
    let scope = DiscoveryScope::resolve(DiscoveryInputs {
        reserved_user_roots,
        roots,
    })?;
    let state = project_root::resolve_state_root(&requested, &config.workspace.state);
    let current = scope.known_read_location(&requested, &state)?;
    let controls = tokio::select! {
        biased;
        event = termination.recv() => return Err(terminated(event)),
        _ = parent.wait() => anyhow::bail!("wait request lost its launching process before admission"),
        result = ControlSnapshot::observe_at(&scope, Some(&current), &config.workspace.state, false) => result?,
    };
    // A directory with two live publications has no implicit winner. Keep
    // the selected exact publication through the entire wait request.
    let selected = controls.history().select(&requested, Some(&directory))?;
    let metadata = match selected {
        Some(HistoryTarget::Live { publication, .. }) => {
            anyhow::ensure!(
                publication.metadata().protocol == runyte::protocol::VERSION,
                "incompatible persistent session cannot accept --wait"
            );
            report_retained_host_logging(arguments);
            publication.metadata().clone()
        }
        target => {
            let project = match target {
                Some(HistoryTarget::Stopped { row }) => row.project_root.clone(),
                None => requested,
                Some(HistoryTarget::Live { .. }) => unreachable!(),
            };
            let layout = scope.initialize_layout(&project, &config.workspace.state)?;
            let location = layout.publication_location()?;
            let mut startup = NativeHostStartup::new(std::env::current_exe()?);
            startup.env = layout.detached_environment()?;
            startup.working_directory = Some(layout.project_root().to_owned());
            startup.config = config_path;
            startup.verbosity = arguments.verbosity;
            startup.log = arguments.log.clone();
            let started =
                start_native_host_with_wait_lifecycle(&location, startup, termination, &parent)
                    .await?;
            if started.disposition() == windows_startup::StartDisposition::ExistingWinner {
                report_retained_host_logging(arguments);
            }
            started.metadata().clone()
        }
    };
    startup_trace.mark(StartupPhase::ProjectResolvedAutomatically);
    let mut client = tokio::select! {
        biased;
        event = termination.recv() => return Err(terminated(event)),
        _ = parent.wait() => anyhow::bail!("wait request lost its launching process before admission"),
        result = connect_control(&metadata) => result?,
    };
    let request = ClientRequest::CreateWait {
        paths: paths.iter().map(|path| encode_path(path)).collect(),
    };
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    tokio::select! {
        biased;
        event = termination.recv() => return Err(terminated(event)),
        _ = parent.wait() => anyhow::bail!("wait request lost its launching process before admission"),
        sent = tokio::time::timeout_at(deadline, client.send(&request)) => {
            sent.context("persistent wait admission timed out")??;
        }
    }
    let admitted = tokio::select! {
        biased;
        event = termination.recv() => return Err(terminated(event)),
        _ = parent.wait() => anyhow::bail!("wait request lost its launching process before admission"),
        answer = tokio::time::timeout_at(deadline, client.recv()) => {
            answer.context("persistent wait admission timed out")??
        }
    };
    let token = match admitted {
        Some(HostResponse::WaitCreated { token, .. }) => token,
        Some(HostResponse::Error { message } | HostResponse::Refused { message }) => {
            anyhow::bail!(message)
        }
        other => anyhow::bail!("persistent host did not create a wait request: {other:?}"),
    };
    let outcome = run_native_wait_until_complete(
        &mut client,
        &metadata,
        token,
        config.editor.mouse,
        termination,
        &parent,
    )
    .await;
    if outcome.is_err() {
        let _ = tokio::time::timeout(
            Duration::from_secs(2),
            client.send(&ClientRequest::CancelWait { token }),
        )
        .await;
    }
    outcome
}

#[cfg(windows)]
async fn start_native_host_with_wait_lifecycle(
    location: &EndpointLocation,
    startup: NativeHostStartup,
    termination: &mut TerminationSignals,
    parent: &ForegroundParentSupervisor,
) -> Result<windows_startup::StartedHost> {
    let mut cancelled = None;
    let started = windows_startup::start_detached_host_cancellable(location, startup, async {
        tokio::select! {
            biased;
            event = termination.recv() => cancelled = Some(terminated(event)),
            _ = parent.wait() => cancelled = Some(anyhow::anyhow!("wait request lost its launching process during host startup")),
        }
        "persistent wait startup cancelled"
    })
    .await;
    if let Some(reason) = cancelled {
        return Err(reason);
    }
    started
}

#[cfg(windows)]
async fn native_wait_status(
    client: &mut runyte::workspace::windows_transport::LocalClient,
    metadata: &runyte::workspace::windows_endpoint::EndpointMetadata,
    token: runyte::protocol::WaitToken,
    termination: &mut TerminationSignals,
    parent: &ForegroundParentSupervisor,
) -> Result<(runyte::protocol::WaitStatus, bool)> {
    use runyte::protocol::{ClientRequest, HostResponse};
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    // Cancelling a LocalClient send closes its writer. Finish this bounded
    // write before checking lifecycle loss, so recovery can read its reply.
    if let Err(error) =
        tokio::time::timeout_at(deadline, client.send(&ClientRequest::WaitStatus { token }))
            .await
            .context("persistent wait status send timed out")
            .and_then(|result| result)
    {
        return recover_native_wait_after_lifecycle_loss(client, metadata, token, error).await;
    }
    loop {
        let response = tokio::select! {
            biased;
            event = termination.recv() => return recover_native_wait_after_lifecycle_loss(client, metadata, token, terminated(event)).await,
            _ = parent.wait() => return recover_native_wait_after_lifecycle_loss(client, metadata, token, anyhow::anyhow!("wait request lost its launching process")).await,
            answer = tokio::time::timeout_at(deadline, client.recv()) => {
                match answer.context("persistent host stopped answering the wait request") {
                    Ok(Ok(response)) => response,
                    Ok(Err(error)) | Err(error) => return recover_native_wait_after_lifecycle_loss(client, metadata, token, error).await,
                }
            },
        };
        match response {
            Some(HostResponse::WaitState {
                token: received,
                status,
                interactive_attached,
            }) if received == token => {
                return Ok((status, interactive_attached));
            }
            Some(HostResponse::Error { message } | HostResponse::Refused { message }) => {
                anyhow::bail!(message)
            }
            Some(_) => {}
            None => anyhow::bail!("persistent host disconnected before wait completion"),
        }
    }
}

#[cfg(windows)]
async fn recover_native_wait_after_lifecycle_loss(
    client: &mut runyte::workspace::windows_transport::LocalClient,
    metadata: &runyte::workspace::windows_endpoint::EndpointMetadata,
    token: runyte::protocol::WaitToken,
    lifecycle_error: anyhow::Error,
) -> Result<(runyte::protocol::WaitStatus, bool)> {
    use runyte::protocol::{ClientRequest, HostResponse, WaitStatus};
    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    let recovery = async {
        let first = tokio::time::timeout(Duration::from_millis(500), async {
            loop {
                match client.recv().await? {
                    Some(HostResponse::WaitState {
                        token: received,
                        status,
                        interactive_attached,
                    }) if received == token => match status {
                        WaitStatus::Pending { .. } => {}
                        WaitStatus::Completed | WaitStatus::Cancelled { .. } => {
                            return Ok::<_, anyhow::Error>(Some((status, interactive_attached)));
                        }
                    },
                    Some(_) => {}
                    None => return Ok(None),
                }
            }
        })
        .await;
        if let Ok(Ok(Some((status, interactive_attached)))) = first {
            return Ok(Some((status, interactive_attached)));
        }
        // Retain the original owner while a fresh control connection queries
        // the exact publication. Never retry a possibly partial send on its
        // poisoned writer.
        let mut query = runyte::workspace::windows_lifecycle::connect_control(metadata).await?;
        query.send(&ClientRequest::WaitStatus { token }).await?;
        loop {
            match query.recv().await? {
                Some(HostResponse::WaitState {
                    token: received,
                    status,
                    interactive_attached,
                }) if received == token => {
                    return Ok(match status {
                        WaitStatus::Completed | WaitStatus::Cancelled { .. } => {
                            Some((status, interactive_attached))
                        }
                        WaitStatus::Pending { .. } => None,
                    });
                }
                Some(HostResponse::Error { message } | HostResponse::Refused { message }) => {
                    anyhow::bail!(message);
                }
                Some(_) => {}
                None => anyhow::bail!("persistent host disconnected before wait recovery"),
            }
        }
    };
    match tokio::time::timeout_at(deadline, recovery).await {
        Ok(Ok(Some(status))) => Ok(status),
        Ok(Ok(None)) => Err(lifecycle_error),
        Ok(Err(error)) => Err(lifecycle_error.context(error)),
        Err(_) => Err(lifecycle_error.context("persistent wait status recovery timed out")),
    }
}

#[cfg(windows)]
async fn run_native_wait_until_complete(
    client: &mut runyte::workspace::windows_transport::LocalClient,
    metadata: &runyte::workspace::windows_endpoint::EndpointMetadata,
    token: runyte::protocol::WaitToken,
    mouse_enabled: bool,
    termination: &mut TerminationSignals,
    parent: &ForegroundParentSupervisor,
) -> Result<()> {
    use runyte::protocol::{HostResponse, WaitStatus};
    loop {
        let (status, interactive_attached) =
            native_wait_status(client, metadata, token, termination, parent).await?;
        match status {
            WaitStatus::Completed => return Ok(()),
            WaitStatus::Cancelled { reason } => anyhow::bail!(reason),
            WaitStatus::Pending { .. } if !interactive_attached => {
                let mut attachment = Box::pin(windows_frontend::attach_exact_for_wait(
                    metadata,
                    termination,
                    mouse_enabled,
                    token,
                ));
                loop {
                    let next = tokio::select! {
                        biased;
                        _ = parent.wait() => Some(Err(anyhow::anyhow!("wait request lost its launching process"))),
                        response = client.recv() => match response? {
                            Some(HostResponse::WaitState { token: received, status: WaitStatus::Completed, .. }) if received == token => Some(Ok(())),
                            Some(HostResponse::WaitState { token: received, status: WaitStatus::Cancelled { reason }, .. }) if received == token => Some(Err(anyhow::anyhow!(reason))),
                            Some(HostResponse::Error { message } | HostResponse::Refused { message }) => Some(Err(anyhow::anyhow!(message))),
                            Some(_) => continue,
                            None => Some(Err(anyhow::anyhow!("persistent host disconnected before wait completion"))),
                        },
                        result = &mut attachment => result.err().map(Err),
                    };
                    if let Some(result) = next {
                        drop(attachment);
                        match result {
                            Ok(()) => return Ok(()),
                            Err(error) => {
                                let (status, _) = recover_native_wait_after_lifecycle_loss(
                                    client, metadata, token, error,
                                )
                                .await?;
                                return match status {
                                    WaitStatus::Completed => Ok(()),
                                    WaitStatus::Cancelled { reason } => anyhow::bail!(reason),
                                    WaitStatus::Pending { .. } => unreachable!(),
                                };
                            }
                        }
                    }
                    break;
                }
                drop(attachment);
                let (status, _) =
                    native_wait_status(client, metadata, token, termination, parent).await?;
                match status {
                    WaitStatus::Completed => return Ok(()),
                    WaitStatus::Cancelled { reason } => anyhow::bail!(reason),
                    WaitStatus::Pending { .. } => anyhow::bail!(
                        "wait editor detached before the requested files were completed"
                    ),
                }
            }
            WaitStatus::Pending { .. } => {
                tokio::select! {
                    biased;
                    event = termination.recv() => {
                        let (status, _) = recover_native_wait_after_lifecycle_loss(client, metadata, token, terminated(event)).await?;
                        return match status {
                            WaitStatus::Completed => Ok(()),
                            WaitStatus::Cancelled { reason } => anyhow::bail!(reason),
                            WaitStatus::Pending { .. } => unreachable!(),
                        };
                    },
                    _ = parent.wait() => {
                        let (status, _) = recover_native_wait_after_lifecycle_loss(client, metadata, token, anyhow::anyhow!("wait request lost its launching process")).await?;
                        return match status {
                            WaitStatus::Completed => Ok(()),
                            WaitStatus::Cancelled { reason } => anyhow::bail!(reason),
                            WaitStatus::Pending { .. } => unreachable!(),
                        };
                    },
                    _ = tokio::time::sleep(Duration::from_millis(100)) => {},
                }
            }
        }
    }
}

#[cfg(windows)]
async fn run_native_persistent(
    arguments: &LaunchArguments,
    startup_trace: &mut StartupTrace,
    termination: &mut TerminationSignals,
    preloaded_config: Option<(Config, Option<PathBuf>)>,
) -> Result<()> {
    anyhow::ensure!(
        arguments.targets.is_empty() && arguments.init.is_none(),
        "persistent attachment does not accept file targets or --init"
    );
    let (config, config_path) = match preloaded_config {
        Some(loaded) => loaded,
        None => Config::load(arguments.config.as_deref())?,
    };
    startup_trace.mark(StartupPhase::ConfigLoaded);
    let directory = std::env::current_dir()?;
    let cwd_file = arguments
        .cwd_file
        .as_ref()
        .map(|path| resolve_cwd_file_path(&directory, path.clone()));
    let cwd_handoff = cwd_file
        .as_deref()
        .map(runyte::cwd_handoff::Prepared::prepare)
        .transpose()
        .context("cannot prepare private PowerShell directory handoff")?;
    let roots = CapturedRoots::capture();
    let mut reserved_user_roots = config_path
        .as_deref()
        .map(|path| config::config_root_for(path, &directory))
        .into_iter()
        .collect::<Vec<_>>();
    if let Some(cache) = external_open::cache_root() {
        reserved_user_roots.push(cache);
    }
    let scope = DiscoveryScope::resolve(DiscoveryInputs {
        reserved_user_roots,
        roots,
    })?;
    let current = if arguments.workspace_selector.is_none() {
        Some(match arguments.project_root.as_deref() {
            Some(root) => resolve_requested_project_root(&directory, root)?,
            None => project_root::discover(&directory, &config.workspace.state)?
                .context(project_root::NO_WORKSPACE_HERE)?,
        })
    } else {
        None
    };
    let selector = arguments
        .workspace_selector
        .as_deref()
        .or(current.as_deref())
        .expect("persistent attachment has a selector or current directory");
    let current_read = current
        .as_deref()
        .map(|project| {
            let state = project_root::resolve_state_root(project, &config.workspace.state);
            scope.known_read_location(project, &state)
        })
        .transpose()?;
    let controls = tokio::select! {
        biased;
        event = termination.recv() => return Err(terminated(event)),
        result = ControlSnapshot::observe_at(&scope, current_read.as_ref(), &config.workspace.state, false) => result?,
    };
    let selected = controls.history().select(selector, Some(&directory))?;
    if let Some(event) = termination.pending_event().await {
        return Err(terminated(event));
    }
    let metadata = match selected {
        Some(HistoryTarget::Live { publication, .. }) => {
            anyhow::ensure!(
                publication.metadata().protocol == runyte::protocol::VERSION,
                "incompatible persistent session cannot be attached"
            );
            report_retained_host_logging(arguments);
            publication.metadata().clone()
        }
        target => {
            let requested = match target {
                Some(HistoryTarget::Stopped { row }) => row.project_root.clone(),
                None => workspace_selector_path(selector, &directory),
                Some(HistoryTarget::Live { .. }) => unreachable!(),
            };
            let layout = scope.initialize_layout(&requested, &config.workspace.state)?;
            let location = layout.publication_location()?;
            let mut startup = NativeHostStartup::new(std::env::current_exe()?);
            startup.env = layout.detached_environment()?;
            startup.working_directory = Some(layout.project_root().to_owned());
            startup.config = config_path;
            startup.verbosity = arguments.verbosity;
            startup.log = arguments.log.clone();
            let started = start_native_host_with_termination(
                &location,
                startup,
                termination,
                "persistent attachment cancelled during native host startup",
            )
            .await?;
            if started.disposition() == windows_startup::StartDisposition::ExistingWinner {
                report_retained_host_logging(arguments);
            }
            started.metadata().clone()
        }
    };
    startup_trace.mark(StartupPhase::ProjectResolvedAutomatically);
    windows_frontend::attach_exact_with_catalog(
        &metadata,
        termination,
        config.editor.mouse,
        &scope,
        &config.workspace.state,
        cwd_handoff.as_ref(),
    )
    .await
}

#[cfg(windows)]
async fn start_native_host_with_termination(
    location: &EndpointLocation,
    startup: NativeHostStartup,
    termination: &mut TerminationSignals,
    reason: &'static str,
) -> Result<windows_startup::StartedHost> {
    let mut cancelled = None;
    let started = windows_startup::start_detached_host_cancellable(location, startup, async {
        cancelled = Some(termination.recv().await);
        reason
    })
    .await;
    if let Some(event) = cancelled {
        let cancellation = terminated(event);
        return match started {
            Err(error) if error.to_string() != reason => Err(cancellation.context(error)),
            _ => Err(cancellation),
        };
    }
    started
}

#[cfg(windows)]
async fn run_native_control_cli(
    arguments: &LaunchArguments,
    startup: &mut StartupTrace,
    termination: &mut TerminationSignals,
) -> Result<()> {
    anyhow::ensure!(
        arguments.project_root.is_none(),
        "--project-root is not available in this workspace mode"
    );
    if arguments.mode == LaunchMode::StopSession {
        anyhow::ensure!(
            arguments.workspace_selector.is_some(),
            "--session-stop on Windows requires an explicit workspace selector"
        );
    }
    // Root selection belongs to this invocation, not to an inferred cwd
    // project. A missing launch directory still permits IDs, names and
    // absolute selectors to reach a complete captured catalog.
    let launch_directory = match std::env::current_dir() {
        Ok(path) => Some(path),
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => return Err(error).context("cannot inspect launch directory"),
    };
    let roots = CapturedRoots::capture();
    let (config, config_path) = Config::load(arguments.config.as_deref())?;
    startup.mark(StartupPhase::ConfigLoaded);
    let mut reserved_user_roots = config_path
        .as_deref()
        .map(|path| {
            // Config::load returns an absolute path. Preserve its canonical
            // parent admission even when the process cwd has disappeared.
            config::config_root_for(path, path.parent().unwrap_or(path))
        })
        .into_iter()
        .collect::<Vec<_>>();
    if let Some(cache) = external_open::cache_root() {
        reserved_user_roots.push(cache);
    }
    let scope = DiscoveryScope::resolve(DiscoveryInputs {
        reserved_user_roots,
        roots,
    })?;
    let mut controls =
        ControlSnapshot::observe(&scope, &config.workspace.state, arguments.include_hidden).await?;
    match arguments.mode {
        LaunchMode::ListSessions => {
            print!(
                "{}",
                format_session_table(controls.history().entries().iter().map(|entry| entry.row()))
            );
        }
        LaunchMode::StopAllSessions => {
            let mut operation = controls.stop_all(arguments.force)?;
            operation.run_to_completion().await;
            report_native_stop_all(operation.report())?;
        }
        LaunchMode::CleanSessions => {
            let cleared = controls.clean()?;
            println!(
                "forgot {cleared} stopped session{}",
                if cleared == 1 { "" } else { "s" }
            );
        }
        LaunchMode::RenameSession => {
            let selector = arguments
                .workspace_selector
                .as_deref()
                .expect("parser set selector");
            let selected = controls.select(UserSelector {
                selector,
                working_directory: launch_directory.as_deref(),
            })?;
            let name = normalize_session_name(
                arguments
                    .workspace_name
                    .as_deref()
                    .expect("parser set name"),
            );
            let outcome = controls.rename(selected, &name).await?;
            if let Some(issue) = outcome.cache_issue {
                eprintln!(
                    "session name changed, but recent history could not be refreshed: {issue}"
                );
            }
        }
        LaunchMode::RestartSession => {
            let discovered = if arguments.workspace_selector.is_none() {
                let directory = launch_directory.as_deref().context(
                    "--session-restart requires a workspace selector or current project",
                )?;
                Some(project_root::discover(directory, &config.workspace.state)?.context(
                    "--session-restart requires a workspace selector or discoverable current project",
                )?)
            } else {
                None
            };
            let selector = arguments
                .workspace_selector
                .as_deref()
                .or(discovered.as_deref())
                .expect("restart has a selector or discovered project");
            let selected = controls.select(UserSelector {
                selector,
                working_directory: launch_directory.as_deref(),
            })?;
            let Some(HistoryTarget::Live { publication, .. }) = controls.history().target(selected)
            else {
                anyhow::bail!("selected persistent session is already stopped");
            };
            let original = publication.metadata().clone();
            let project = original.project_root()?;
            let state = project_root::resolve_state_root(&project, &config.workspace.state);
            let layout = ResolvedLayout::from_scope(scope.clone(), &project, state)?;
            let location = layout.publication_location()?;
            let mut replacement = NativeHostStartup::new(std::env::current_exe()?);
            replacement.env = layout.detached_environment()?;
            replacement.working_directory = Some(project);
            replacement.config = config_path;
            replacement.verbosity = arguments.verbosity;
            replacement.log = arguments.log.clone();
            windows_startup::preflight_detached_host(&location, &replacement).await?;
            if let Some(event) = termination.pending_event().await {
                return Err(terminated(event));
            }
            let ready = location.observe_ready()?.context(
                "selected publication is no longer ready at its configured restart location",
            )?;
            anyhow::ensure!(
                ready.metadata() == &original,
                "selected publication changed or belongs to another native namespace; restart refused"
            );
            let stopped = controls.stop(selected, arguments.force).await.context(
                "restart did not start a replacement because selected stop was not confirmed",
            )?;
            if original.protocol != runyte::protocol::VERSION {
                eprintln!(
                    "force-stopped persistent session process {} (protocol {}); its protected live state was discarded",
                    original.process.pid, original.protocol
                );
            }
            for issue in stopped.cleanup_issues {
                eprintln!("session stopped; observation cleanup is incomplete: {issue}");
            }
            if let Some(event) = termination.pending_event().await {
                return Err(terminated(event)
                    .context("selected persistent session stopped; replacement was not started"));
            }
            let started = start_native_host_with_termination(
                &location,
                replacement,
                termination,
                "persistent session restart cancelled during native host startup",
            )
            .await
            .context("selected persistent session stopped; replacement did not become ready")?;
            if started.disposition() == windows_startup::StartDisposition::ExistingWinner {
                eprintln!(
                    "selected persistent session stopped; another native host won its configured publication"
                );
                report_retained_host_logging(arguments);
            }
        }
        LaunchMode::StopSession => {
            let selector = arguments
                .workspace_selector
                .as_deref()
                .expect("checked explicit selector");
            let selected = controls.select(UserSelector {
                selector,
                working_directory: launch_directory.as_deref(),
            })?;
            let incompatible = match controls.history().target(selected) {
                Some(HistoryTarget::Live { publication, row })
                    if row.incompatible_protocol.is_some() =>
                {
                    Some((
                        publication.metadata().process.pid,
                        publication.metadata().protocol,
                    ))
                }
                _ => None,
            };
            let outcome = controls.stop(selected, arguments.force).await?;
            if let Some((pid, protocol)) = incompatible {
                eprintln!(
                    "force-stopped persistent session process {pid} (protocol {protocol}); its protected live state was discarded"
                );
            }
            for issue in outcome.cleanup_issues {
                eprintln!("session stopped; observation cleanup is incomplete: {issue}");
            }
        }
        _ => unreachable!("native CLI dispatch selected a control mode"),
    }
    Ok(())
}

#[cfg(windows)]
fn report_native_stop_all(report: StopAllReport) -> Result<()> {
    for issue in &report.cleanup_issues {
        eprintln!("session stopped; observation cleanup is incomplete: {issue}");
    }
    if report.omitted_cleanup_details > 0 {
        eprintln!(
            "{} further observation cleanup detail(s) omitted",
            report.omitted_cleanup_details
        );
    }
    if report.failed == 0 && report.unknown == 0 {
        println!(
            "stopped {} session{}",
            report.stopped,
            if report.stopped == 1 { "" } else { "s" }
        );
        return Ok(());
    }
    let mut message = format!(
        "stopped {} of {} running sessions; {} failed, {} outcome(s) unknown",
        report.stopped, report.total, report.failed, report.unknown
    );
    for detail in report.failures {
        message.push('\n');
        message.push_str(&detail);
    }
    if report.omitted_failure_details > 0 {
        message.push_str(&format!(
            "\n{} further failure detail(s) omitted",
            report.omitted_failure_details
        ));
    }
    if let Some(admitted) = report.admitted_summary {
        message.push_str("\noutcome unknown for ");
        message.push_str(&admitted);
    }
    anyhow::bail!(message)
}

#[cfg(unix)]
async fn list_sessions(state: &Path, include_hidden: bool) -> Result<()> {
    let workspaces = if include_hidden {
        known_workspaces_all_namespaces(state).await?
    } else {
        known_workspaces(state).await?
    };
    print!("{}", format_session_table(workspaces.iter()));
    Ok(())
}

#[cfg(any(unix, windows))]
fn format_session_table<'a>(workspaces: impl IntoIterator<Item = &'a WorkspaceRow>) -> String {
    let workspaces = workspaces.into_iter().collect::<Vec<_>>();
    let width = abbreviated_id_width(workspaces.iter().map(|workspace| workspace.id.as_str()));
    let rows = workspaces
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
                    .plugin_jobs
                    .map_or_else(String::new, |count| count.to_string()),
                workspace
                    .activity_leases
                    .map_or_else(String::new, |count| count.to_string()),
                workspace
                    .interactive_attached
                    .map_or_else(String::new, |attached| {
                        if attached { "yes" } else { "no" }.to_owned()
                    }),
            ]
            .map(|cell| display_session_table_cell(&cell))
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
        "JOBS".to_owned(),
        "ACTIVITIES".to_owned(),
        "TUI".to_owned(),
    ];
    let mut widths = [0_usize; 10];
    for row in std::iter::once(&headings).chain(rows.iter()) {
        for (index, value) in row.iter().enumerate() {
            widths[index] =
                widths[index].max(unicode_width::UnicodeWidthStr::width(value.as_str()));
        }
    }
    let mut output = String::new();
    append_workspace_row(&mut output, &headings, &widths);
    append_workspace_row(&mut output, &widths.map(|width| "-".repeat(width)), &widths);
    for row in rows {
        append_workspace_row(&mut output, &row, &widths);
    }
    output
}

#[cfg(any(unix, windows))]
fn display_session_table_cell(text: &str) -> String {
    let mut displayed = String::with_capacity(text.len());
    for character in text.chars() {
        if character.is_control() {
            displayed.extend(character.escape_default());
        } else {
            displayed.push(character);
        }
    }
    displayed
}

#[cfg(any(unix, windows))]
fn append_workspace_row(output: &mut String, row: &[String; 10], widths: &[usize; 10]) {
    let cells = std::array::from_fn::<_, 10, _>(|index| pad_table_cell(&row[index], widths[index]));
    output.push_str(&cells.join("  "));
    output.push('\n');
}

#[cfg(any(unix, windows))]
fn pad_table_cell(value: &str, width: usize) -> String {
    let used = unicode_width::UnicodeWidthStr::width(value);
    format!("{value}{}", " ".repeat(width.saturating_sub(used)))
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
    include_hidden: bool,
) -> Result<()> {
    if include_hidden {
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
#[allow(clippy::too_many_arguments)]
async fn wait_for_completion(
    client: &mut LocalClient,
    endpoint: &LocalEndpoint,
    mouse_enabled: bool,
    token: WaitToken,
    termination: &mut TerminationSignals,
    terminal_loss: &mut TerminalLoss,
    launching_parent: &HostSupervisor,
    parent_routed: bool,
) -> Result<()> {
    let mut test_status_barrier =
        std::env::var_os("RUNYTE_TEST_WAIT_STATUS_BARRIER").map(PathBuf::from);
    loop {
        client.send(&ClientRequest::WaitStatus { token }).await?;
        wait_at_test_status_barrier(&mut test_status_barrier).await?;
        let response = tokio::select! {
            biased;
            signal = termination.recv() => return Err(terminated(signal)),
            parent = launching_parent.recv() => {
                let error = parent.map_or_else(
                    |error| error.context("failed while watching the wait client's launching process"),
                    |()| anyhow::anyhow!("wait request lost its launching process before completion"),
                );
                return recover_wait_after_lifecycle_loss(client, token, true, error).await;
            }
            loss = terminal_loss.recv() => {
                loss?;
                let error = terminal_loss_error(termination).await;
                if error.downcast_ref::<TerminatedBySignal>().is_some() {
                    return Err(error);
                }
                return recover_wait_after_lifecycle_loss(client, token, true, error).await;
            }
            response = async {
                if parent_routed { tokio::time::timeout(Duration::from_secs(3), client.recv()).await
                    .context("owning host stopped answering the parent wait request")? }
                else { client.recv().await }
            } => response?,
        };
        match response {
            Some(HostResponse::WaitState {
                token: response_token,
                status,
                interactive_attached,
            }) if response_token == token => match status {
                WaitStatus::Completed => return Ok(()),
                WaitStatus::Cancelled { reason } => anyhow::bail!(reason),
                WaitStatus::Pending { .. } if !interactive_attached && !parent_routed => {
                    return attach_for_wait(
                        endpoint,
                        mouse_enabled,
                        token,
                        client,
                        termination,
                        terminal_loss,
                        launching_parent,
                    )
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
            biased;
            signal = termination.recv() => return Err(terminated(signal)),
            parent = launching_parent.recv() => {
                let error = parent.map_or_else(
                    |error| error.context("failed while watching the wait client's launching process"),
                    |()| anyhow::anyhow!("wait request lost its launching process before completion"),
                );
                return recover_wait_after_lifecycle_loss(client, token, false, error).await;
            }
            loss = terminal_loss.recv() => {
                loss?;
                let error = terminal_loss_error(termination).await;
                if error.downcast_ref::<TerminatedBySignal>().is_some() {
                    return Err(error);
                }
                return recover_wait_after_lifecycle_loss(client, token, false, error).await;
            }
            _ = tokio::time::sleep(Duration::from_millis(100)) => {}
        }
    }
}

/// Gives process-level tests a one-shot acknowledgement after the wait client
/// has sent a status request but before it can consume the reply. This makes a
/// completion-versus-launcher-loss race reproducible without elapsed-time
/// guesses. Ordinary clients never set the test-only environment variable.
#[cfg(unix)]
async fn wait_at_test_status_barrier(barrier: &mut Option<PathBuf>) -> Result<()> {
    let Some(path) = barrier.take() else {
        return Ok(());
    };
    let (ready, release) = runyte::test_support::wait_status_barrier_paths(path);
    fs::write(&ready, []).with_context(|| {
        format!(
            "cannot publish wait-status test barrier {}",
            ready.display()
        )
    })?;
    let deadline = Instant::now() + Duration::from_secs(5);
    while !release.exists() {
        anyhow::ensure!(
            Instant::now() < deadline,
            "wait-status test barrier was not released at {}",
            release.display()
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    Ok(())
}

/// Resolves client lifecycle loss against the host's durable wait state.
///
/// A status request is already in flight while `wait_for_completion` waits for
/// its response. Read that answer first, then ask once more if it was pending:
/// explicit completion that reached the host before terminal loss remains a
/// success, while a still-pending request is released by `run_wait`'s ordinary
/// error cleanup.
#[cfg(unix)]
async fn recover_wait_after_lifecycle_loss(
    client: &mut LocalClient,
    token: WaitToken,
    mut status_in_flight: bool,
    terminal_error: anyhow::Error,
) -> Result<()> {
    let mut terminal_error = Some(terminal_error);
    let recovery = async {
        for attempt in 0..2 {
            if !status_in_flight {
                client.send(&ClientRequest::WaitStatus { token }).await?;
            }
            status_in_flight = false;
            match client.recv().await? {
                Some(HostResponse::WaitState {
                    token: response_token,
                    status: WaitStatus::Completed,
                    ..
                }) if response_token == token => return Ok(()),
                Some(HostResponse::WaitState {
                    token: response_token,
                    status: WaitStatus::Cancelled { reason },
                    ..
                }) if response_token == token => anyhow::bail!(reason),
                Some(HostResponse::WaitState {
                    token: response_token,
                    status: WaitStatus::Pending { .. },
                    ..
                }) if response_token == token && attempt == 0 => {}
                Some(HostResponse::WaitState {
                    token: response_token,
                    status: WaitStatus::Pending { .. },
                    ..
                }) if response_token == token => {
                    return Err(terminal_error
                        .take()
                        .expect("terminal error is consumed only by the final response"));
                }
                Some(HostResponse::Error { message } | HostResponse::Refused { message }) => {
                    anyhow::bail!(message)
                }
                Some(response) => {
                    return Err(terminal_error
                        .take()
                        .expect("terminal error is consumed only by a terminal response")
                        .context(format!(
                            "wait lifecycle status recovery returned {response:?}"
                        )));
                }
                None => {
                    return Err(terminal_error
                        .take()
                        .expect("terminal error is consumed only by a terminal response")
                        .context("workspace host disconnected during wait lifecycle recovery"));
                }
            }
        }
        unreachable!("the bounded wait-lifecycle recovery loop always returns")
    };
    match tokio::time::timeout(WAIT_LIFECYCLE_RECOVERY_BUDGET, recovery).await {
        Ok(result) => result,
        Err(_) => Err(terminal_error
            .take()
            .expect("timed-out recovery has not consumed its terminal error")
            .context("workspace host did not answer wait lifecycle status recovery")),
    }
}

async fn context_timeout(delay: Option<Duration>) {
    match delay {
        Some(delay) => tokio::time::sleep(delay).await,
        None => std::future::pending().await,
    }
}

/// Lets a native process fixture stop after service ownership is complete and
/// then inject a frontend setup failure. The error remains inside the ordinary
/// standalone outcome so context, catalog and plugin cleanup are all joined.
#[cfg(all(windows, debug_assertions))]
async fn wait_at_post_service_failure_barrier() -> Result<()> {
    let Some(base) = std::env::var_os("RUNYTE_TEST_POST_SERVICE_FAILURE_BARRIER") else {
        return Ok(());
    };
    let base = PathBuf::from(base);
    anyhow::ensure!(
        base.is_absolute(),
        "RUNYTE_TEST_POST_SERVICE_FAILURE_BARRIER must name an absolute path"
    );
    let (ready, release) = runyte::test_support::wait_status_barrier_paths(base);
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&ready)
        .with_context(|| {
            format!(
                "cannot publish post-service failure barrier {}",
                ready.display()
            )
        })?;
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if release.try_exists().with_context(|| {
            format!(
                "cannot inspect post-service failure barrier {}",
                release.display()
            )
        })? {
            anyhow::bail!("injected post-service frontend failure after service startup");
        }
        if Instant::now() >= deadline {
            anyhow::bail!(
                "post-service failure barrier timed out waiting for {}",
                release.display()
            );
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

struct HostServices {
    #[cfg(windows)]
    native_catalog: Option<runyte::workspace::WorkspaceServiceHandle>,
    #[cfg(windows)]
    native_catalog_owner: Option<runyte::workspace::windows_service::WorkspaceServiceOwner>,
    #[cfg(windows)]
    // The event receiver stays with the owner and enters the host loop directly.
    native_catalog_events: Option<tokio::sync::mpsc::Receiver<runyte::workspace::WorkspaceEvent>>,
    context_events: tokio::sync::mpsc::Receiver<runyte::workspace::context::Event>,
    pipe_events: tokio::sync::mpsc::Receiver<runyte::pipe::Completion>,
    plugin_events: Option<tokio::sync::mpsc::Receiver<runyte::plugin::Event>>,
    syntax_events: SyntaxEvents,
    git_events: Option<tokio::sync::mpsc::Receiver<GitServiceEvent>>,
    language_servers: LspHandle,
    lsp_events: lsp::LspEvents,
    file_picker_events: tokio::sync::mpsc::Receiver<runyte::file_picker::FilePickerEvent>,
    workspace_search_events:
        tokio::sync::mpsc::Receiver<runyte::workspace_search::WorkspaceSearchEvent>,
    file_monitor: runyte::file_monitor::FileMonitorHandle,
    file_monitor_events: tokio::sync::mpsc::Receiver<runyte::buffer::FileObservationEvent>,
    git_monitor: runyte::git_monitor::GitMonitorHandle,
    git_monitor_events: tokio::sync::mpsc::Receiver<runyte::git_monitor::GitInvalidation>,
    workspace_events: Option<tokio::sync::mpsc::Receiver<HostEvent>>,
    /// Output from every child running on a terminal pane.
    ///
    /// Held here rather than inside the editor so a loop can wait on it beside
    /// its other sources without keeping the editor mutably borrowed for the
    /// whole of a `select!`.
    terminal_events: TerminalEvents,
}

#[cfg(windows)]
struct NativeCatalogConfig {
    scope: DiscoveryScope,
    current: runyte::workspace::windows_location::KnownReadLocation,
    current_layout: Option<runyte::workspace::windows_location::ResolvedLayout>,
    configured_state: PathBuf,
    parent_attach: Option<runyte::workspace::windows_service::ParentAttachStartup>,
}

#[cfg(windows)]
impl NativeCatalogConfig {
    fn from_layout(
        layout: &runyte::workspace::windows_location::ResolvedLayout,
        configured_state: PathBuf,
        parent_attach: runyte::workspace::windows_service::ParentAttachStartup,
    ) -> Self {
        Self {
            scope: layout.discovery_scope().clone(),
            current: layout.read_location(),
            current_layout: Some(layout.clone()),
            configured_state,
            parent_attach: Some(parent_attach),
        }
    }
}

#[cfg(windows)]
impl HostServices {
    async fn shutdown_native_catalog(&mut self) -> Result<()> {
        if let Some(owner) = self.native_catalog_owner.as_mut() {
            owner.shutdown().await?;
        }
        self.native_catalog_owner = None;
        self.native_catalog = None;
        self.native_catalog_events = None;
        Ok(())
    }
}

/// A service's terminal event is observed once; later selects await useful work.
async fn receive_service_event<T>(
    service: &'static str,
    ended: &std::collections::HashSet<&'static str>,
    event: impl std::future::Future<Output = Option<T>>,
) -> Option<T> {
    if ended.contains(service) {
        return std::future::pending().await;
    }
    event.await
}

async fn receive_optional_service_event<T>(
    events: &mut Option<tokio::sync::mpsc::Receiver<T>>,
) -> Option<T> {
    let event = match events.as_mut() {
        Some(events) => events.recv().await,
        None => std::future::pending().await,
    };
    if event.is_none() {
        *events = None;
    }
    event
}

fn start_host_services(
    app: &mut WorkspaceHost,
    startup: &mut StartupTrace,
    config_path: Option<&Path>,
    persistent: bool,
    #[cfg(windows)] native_catalog: Option<NativeCatalogConfig>,
) -> Result<HostServices> {
    // Editor mode starts none of the services that assume a workspace:
    // no agent context endpoint, session catalog, Git, language server or
    // plugin. Their receivers are left closed or absent, which the event
    // loop already treats as a service that is not running.
    let editor = app.is_editor_mode();
    // Windows context initialization is the only fallible service setup below.
    // Complete it before spawning or transferring ownership of any other
    // service so an initialization error has no background owners to abandon.
    #[cfg(unix)]
    let context_events = if editor {
        tokio::sync::mpsc::channel(1).1
    } else {
        app.start_context(if persistent {
            runyte::workspace::context::storage::HostMode::Persistent
        } else {
            runyte::workspace::context::storage::HostMode::Standalone
        })
    };
    #[cfg(windows)]
    let context_events = if editor {
        tokio::sync::mpsc::channel(1).1
    } else {
        app.start_context(if persistent {
            runyte::workspace::context::storage::HostMode::Persistent
        } else {
            runyte::workspace::context::storage::HostMode::Standalone
        })?
    };
    #[cfg(all(not(unix), not(windows)))]
    let context_events = tokio::sync::mpsc::channel(1).1;
    #[cfg(windows)]
    let (native_catalog_handle, native_catalog_owner, native_catalog_events) =
        spawn_native_catalog(app, native_catalog);
    let git_events = if editor { None } else { start_git_service(app) };
    // The manager is spawned either way so the loop has its channel, but
    // editor mode never attaches it: no document is offered to it and no
    // server process starts.
    let (language_servers, lsp_events) =
        lsp::spawn(app.config.lsp.clone(), app.project_root.clone());
    startup.mark(StartupPhase::LspManagerSpawned);
    if !editor {
        #[cfg(any(unix, windows))]
        app.configure_lsp_trust(
            runyte::external_open::cache_root().map(|root| root.join("lsp-trust")),
        );
        app.attach_lsp(language_servers.clone());
    }
    let (syntax_worker, syntax_events) = syntax::spawn_background(Arc::clone(&app.registry));
    app.attach_syntax_worker(syntax_worker);
    let (file_scanner, file_picker_events) = file_picker::scanner();
    app.attach_file_scanner(file_scanner);
    let (workspace_search, workspace_search_events) = runyte::workspace_search::spawn();
    app.attach_workspace_search(workspace_search);
    let (mut file_monitor, file_monitor_events) = file_monitor::spawn();
    file_monitor.sync(app.file_monitor_requests());
    let (mut git_monitor, git_monitor_events) = git_monitor::spawn();
    git_monitor.sync(app.git_monitor_repository());
    app.attach_word_index(word_index::spawn());
    #[cfg(unix)]
    let workspace_events = if editor {
        None
    } else {
        Some(start_workspace_service(app, config_path))
    };
    #[cfg(not(unix))]
    let workspace_events = None;
    let terminal_events = app
        .take_terminal_events()
        .expect("terminal output is claimed once, when services start");
    let plugin_events = if editor { None } else { app.start_plugins() };
    let pipe_events = app.start_pipe_service();
    #[cfg(windows)]
    let _ = config_path;
    #[cfg(all(not(unix), not(windows)))]
    let _ = (config_path, persistent);
    Ok(HostServices {
        #[cfg(windows)]
        native_catalog: native_catalog_handle,
        #[cfg(windows)]
        native_catalog_owner,
        #[cfg(windows)]
        native_catalog_events,
        context_events,
        pipe_events,
        plugin_events,
        syntax_events,
        git_events,
        language_servers,
        lsp_events,
        file_picker_events,
        workspace_search_events,
        file_monitor,
        file_monitor_events,
        git_monitor,
        git_monitor_events,
        workspace_events,
        terminal_events,
    })
}

/// The native catalog configuration for a standalone editor in the
/// workspace it now serves, or `None` with the reason reported.
#[cfg(windows)]
fn standalone_native_catalog_config(
    app: &mut WorkspaceHost,
    reserved_user_roots: &[PathBuf],
) -> Option<NativeCatalogConfig> {
    let project_root = app.project_root.clone();
    let state_root = app.state_root.clone();
    DiscoveryScope::resolve(DiscoveryInputs {
        reserved_user_roots: reserved_user_roots.to_vec(),
        roots: CapturedRoots::capture(),
    })
    .and_then(|scope| {
        let current = scope.known_read_location(&project_root, &state_root)?;
        Ok(NativeCatalogConfig {
            scope,
            current,
            current_layout: None,
            configured_state: app.config.workspace.state.clone(),
            parent_attach: None,
        })
    })
    .map(Some)
    .unwrap_or_else(|error| {
        app.report_host_error(format!("native session catalog is unavailable: {error}"));
        None
    })
}

/// Starts the native catalog described by `config` and attaches it, so the
/// session controls it backs become available.
#[cfg(windows)]
fn spawn_native_catalog(
    app: &mut WorkspaceHost,
    config: Option<NativeCatalogConfig>,
) -> (
    Option<runyte::workspace::WorkspaceServiceHandle>,
    Option<runyte::workspace::windows_service::WorkspaceServiceOwner>,
    Option<tokio::sync::mpsc::Receiver<runyte::workspace::WorkspaceEvent>>,
) {
    let Some(config) = config else {
        return (None, None, None);
    };
    let NativeCatalogConfig {
        scope,
        current,
        current_layout,
        configured_state,
        parent_attach,
    } = config;
    let spawned = match (parent_attach, current_layout) {
        (Some(parent_attach), Some(layout)) => {
            runyte::workspace::windows_service::WorkspaceServiceOwner::spawn_with_current_layout(
                layout,
                configured_state,
                parent_attach,
            )
        }
        (Some(parent_attach), None) => {
            runyte::workspace::windows_service::WorkspaceServiceOwner::spawn_with_parent_attach(
                scope,
                Some(current),
                configured_state,
                parent_attach,
            )
        }
        (None, _) => runyte::workspace::windows_service::WorkspaceServiceOwner::spawn(
            scope,
            Some(current),
            configured_state,
        ),
    };
    match spawned {
        Ok((handle, owner, events)) => {
            app.attach_workspace_service(handle.clone());
            (Some(handle), Some(owner), Some(events))
        }
        Err(error) => {
            app.report_host_error(format!("native session catalog could not start: {error}"));
            (None, None, None)
        }
    }
}

fn start_git_service(
    app: &mut WorkspaceHost,
) -> Option<tokio::sync::mpsc::Receiver<GitServiceEvent>> {
    let provider = GitCliProvider::from_environment()?;
    let (service, events) = GitService::spawn(provider);
    app.attach_git_service(service);
    Some(events)
}

#[cfg(unix)]
fn start_workspace_service(
    app: &mut WorkspaceHost,
    config_path: Option<&Path>,
) -> tokio::sync::mpsc::Receiver<HostEvent> {
    let (service, mut events) = WorkspaceService::spawn(
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
    receiver
}

#[cfg(not(windows))]
fn write_cwd_file(path: &Path, directory: &Path) -> Result<()> {
    let mut contents = directory.as_os_str().as_encoded_bytes().to_vec();
    if cfg!(unix) {
        contents.push(0);
    }
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

#[cfg(not(any(unix, windows)))]
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

#[cfg_attr(not(unix), allow(dead_code))]
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

/// Some Windows Terminal builds send an empty bracketed paste for an image.
/// A semantic paste event avoids interpreting it as the next key in a pending
/// command or as a configured binding. Terminals and overlays own their paste.
#[cfg(windows)]
fn is_empty_windows_image_paste(event: &CrosstermEvent, app: &App) -> bool {
    matches!(event, CrosstermEvent::Paste(text) if text.is_empty())
        && app.active_terminal().is_none()
        && !app.has_input_overlay()
        && app.mode != runyte::command::Mode::Command
}

#[cfg(windows)]
fn convert_windows_event(event: CrosstermEvent, app: &App) -> Result<Option<InputEvent>> {
    if is_empty_windows_image_paste(&event, app) {
        Ok(Some(InputEvent::ClipboardPaste))
    } else {
        Ok(convert_event(event)?)
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
    #[cfg(windows)]
    _console_mode: runyte::tui::windows_input::ConsoleMode,
    #[cfg(windows)]
    keyboard_mode: Option<runyte::tui::windows_input::KeyboardMode>,
    mouse_enabled: bool,
    #[cfg_attr(not(unix), allow(dead_code))]
    keyboard_enhancement: bool,
}

/// Draws the stable presentation used while the first editor state is built.
///
/// This screen covers file loading. The first editor frame can show ordinary
/// text colours while its independently scheduled syntax is still parsing.
fn present_startup_screen() -> Result<()> {
    let mut output = stdout();
    write_startup_screen(&mut output).context("failed to present startup screen")
}

fn write_startup_screen(output: &mut impl Write) -> io::Result<()> {
    output
        .queue(SetAttribute(Attribute::Reset))?
        .queue(Hide)?
        .queue(Clear(ClearType::All))?
        .queue(MoveTo(0, 0))?
        .queue(Print("Runyte"))?
        .queue(MoveTo(0, 2))?
        .queue(Print("Opening workspace…"))?;
    output.flush()
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
        #[cfg(windows)]
        let console_mode = runyte::tui::windows_input::ConsoleMode::capture()?;
        enable_raw_mode().context("failed to enable terminal raw mode")?;
        #[cfg(windows)]
        if let Err(error) = console_mode.enable_vt() {
            let _ = disable_raw_mode();
            return Err(error).context("failed to enable Windows VT input");
        }
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
        #[allow(unused_mut)]
        let mut guard = Self {
            mouse_enabled,
            keyboard_enhancement,
            #[cfg(windows)]
            _console_mode: console_mode,
            #[cfg(windows)]
            keyboard_mode: None,
        };
        // Crossterm's native mouse setup replaces the whole input mode. Restore
        // VT input before accepting any editor input; guard owns rollback.
        #[cfg(windows)]
        guard._console_mode.enable_vt()?;
        #[cfg(windows)]
        {
            guard.keyboard_mode = Some(runyte::tui::windows_input::KeyboardMode::enable()?);
        }
        Ok(guard)
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        #[cfg(windows)]
        drop(self.keyboard_mode.take());
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
#[cfg_attr(not(unix), allow(dead_code))]
fn report_retained_logging(verbosity: u8, log: Option<&Path>) {
    if verbosity == 0 && log.is_none() {
        return;
    }
    eprintln!(
        "runyte: the running session kept its own log level and destination; \
restart it with --session-restart to change them"
    );
}

#[cfg_attr(not(unix), allow(dead_code))]
fn report_retained_host_logging(arguments: &LaunchArguments) {
    report_retained_logging(arguments.verbosity, arguments.log.as_deref());
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
    project_root: Option<&Path>,
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
    let abbreviated = project_root.map(|root| {
        let workspace = workspace_id(root);
        workspace
            .get(..ABBREVIATED_LOG_WORKSPACE_ID)
            .unwrap_or(&workspace)
            .to_owned()
    });
    let settings = diagnostic_log::Settings::new(level, role).with_workspace(abbreviated);
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
        --init DIRECTORY Make DIRECTORY a workspace and exit without opening it
    -v, --verbose        Raise the diagnostic log level; repeat for more
        --log PATH       Write the diagnostic log to PATH instead
    -h, --help           Print help
    -V, --version        Print version

MODES:
    Runyte runs in one of three modes, chosen by a flag or by the mode
    setting, ide by default. The launch directory never changes the mode.

        --window         Open the experimental GPUI window (native feature)
        --editor         Edit files and directories with no workspace: no Git,
                         language servers, MCP, plugins or terminals. Running
                         the binary as runed is the same as runyte --editor
        --ide            Work in the workspace found from the launch directory,
                         with Git, language servers, MCP, plugins and terminals
    -a, --mux [WORKSPACE]
                         ide in a persistent session that outlives the TUI.
                         Attach to the selected or current session, starting it
                         if needed. A WORKSPACE directory that is not yet a
                         workspace becomes one
        --wait FILE...   Open through a persistent session and wait for every
                         requested buffer to complete. On Windows, an ordinary
                         shell uses the current project's session; an
                         authenticated integrated terminal uses its parent

    A workspace is a Git repository or a directory holding .runyte, found from
    the launch directory or above it. Without one, ide and mux refuse; create
    one with runyte --init DIRECTORY. As root only editor mode runs; for system
    files set SUDO_EDITOR=runed and use sudoedit.

AGENT CONTEXT:
        mcp [--identity NAME] [--timeout SECONDS]
                         Run the built-in MCP stdio server. Grant permissions
                         in the editor with :mcp [identity].
                         Use runyte -- mcp to open a file named mcp.
        --context-list --json
                         List live context-enabled workspaces as versioned JSON
                         without attaching; --include-hidden includes isolated
                         environments. Does not imply content permission.

PERSISTENT SESSIONS:
    A persistent session is the durable local process and retained editor state
    associated with one workspace. CLI listing also works from ide mode;
    session commands inside the editor need mux mode.
    Windows CLI supports list, rename, selected stop, stop-all, clean and restart
    for native persistent sessions. The standalone session manager provides
    list, rename and stop controls. Stop requires WORKSPACE on the Windows CLI.
    Foreground --serve follows its launching process; direct attachment is
    available. Restart checks detached-launch capability before stopping its
    selected session.

    WORKSPACE selects a session by ID, unambiguous ID prefix, persistent name,
    or directory, so a session is reachable from anywhere.

        --serve          Keep a persistent session alive in the foreground
    -l, --session-list   List running and recently visited sessions
    -s, --session-stop [WORKSPACE]
                         Stop the selected or current session
        --session-stop-all
                         Stop every running session in the current environment
        --include-hidden With session-list or session-stop-all, also include
                         live sessions started in isolated Runyte environments
        --session-clean  Forget every stopped session
        --session-restart [WORKSPACE]
                         Replace the selected or current running session using
                         the supplied config and logging options, without attaching
        --session-rename WORKSPACE NAME
                         Rename a persistent session
    -f, --force          With stop/stop-all/restart, discard protected buffers,
                         waiters, and live terminal children

DIAGNOSTICS:
    Runyte keeps a small local log of warnings, errors, and, when asked, more
    detailed lifecycle events. The process that owns editor state owns the
    file: an ide-mode editor writes .runyte/standalone-<pid>.log, a persistent
    session writes .runyte/host.log, and editor mode keeps none unless --log
    names one. At most 4 MiB is kept in the active file
    and 4 MiB in one previous file beside it. A standalone launch keeps the
    four newest logs left by exited standalone processes and removes older
    active and previous files without touching a live owner's log.

    The default level records warnings and errors. Each -v raises it through
    info, debug, and trace, and stops at trace. --log PATH selects another
    destination; a path that cannot be written is a startup error, while an
    unwritable default only degrades logging. On Unix, a path already owned by
    another running Runyte process is refused.

    In persistent mode these are properties of session startup. --serve,
    --session-restart, and the launch that starts a missing session pass them
    on; attaching to a running session leaves its logging alone and says so.
    Inside the editor, :log-open shows the log of the process that owns the
    workspace and :service-health names its owner, level, and path.

    Records never contain document text, selections, clipboard or terminal
    contents, environment values, or language-server message bodies. They do
    contain local paths and process metadata, so review a log before sharing
    it.

TARGETS:
    (no target)          Open the Runyte about page
    DIRECTORY            Open DIRECTORY in the explorer inside the workspace
                         discovered from the launch directory
    +LINE[:COLUMN] FILE  Open FILE and place its caret at a one-based position
    -- FILE...           Treat every remaining argument as a literal path

    Naming a target runs ide or editor mode in this process, so its relative
    path and caret position keep their ordinary meaning: mode: mux changes
    only a bare runyte, and --mux reads its argument as a workspace rather
    than a file. --wait uses a persistent session on Unix and Windows.

:quit-here moves the shell to the editor's directory on exit; it requires the
runyte() shell function documented in README.md.

Inside the editor press Space+? for the complete key reference."
    );
}

#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn ended_services_deliver_queued_work_and_then_stop_waking_the_loop() {
        use super::{note_ended_service, receive_optional_service_event, receive_service_event};
        let (sender, mut receiver) = tokio::sync::mpsc::channel(2);
        sender.send(7).await.unwrap();
        drop(sender);
        let mut ended = std::collections::HashSet::new();
        assert_eq!(
            receive_service_event("worker", &ended, receiver.recv()).await,
            Some(7)
        );
        assert_eq!(
            receive_service_event("worker", &ended, receiver.recv()).await,
            None
        );
        note_ended_service(&mut ended, "worker");
        tokio::select! {
            biased;
            _ = receive_service_event("worker", &ended, receiver.recv()) => panic!("ended service woke again"),
            _ = std::future::ready(()) => {}
        }
        assert!(
            tokio::time::timeout(
                std::time::Duration::from_millis(10),
                receive_service_event("worker", &ended, receiver.recv())
            )
            .await
            .is_err()
        );
        let (sender, receiver) = tokio::sync::mpsc::channel(2);
        sender.send(9).await.unwrap();
        drop(sender);
        let mut optional = Some(receiver);
        assert_eq!(receive_optional_service_event(&mut optional).await, Some(9));
        assert_eq!(receive_optional_service_event(&mut optional).await, None);
        assert!(optional.is_none(), "closed optional receiver was retained");
        assert!(
            tokio::time::timeout(
                std::time::Duration::from_millis(10),
                receive_optional_service_event(&mut optional)
            )
            .await
            .is_err()
        );
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "requires an isolated native console; run under the ConPTY acceptance harness"]
    fn windows_console_paste_and_restoration() {
        use windows_sys::Win32::System::Console::*;
        let input = unsafe { GetStdHandle(STD_INPUT_HANDLE) };
        let mode = || {
            let mut value = 0;
            assert_ne!(unsafe { GetConsoleMode(input, &mut value) }, 0);
            value
        };
        let original = mode();
        let output = unsafe { GetStdHandle(STD_OUTPUT_HANDLE) };
        let marker: Vec<u16> = "Runyte restoration marker".encode_utf16().collect();
        let coordinate = COORD { X: 0, Y: 10 };
        let mut written = 0;
        assert_ne!(
            unsafe {
                WriteConsoleOutputCharacterW(
                    output,
                    marker.as_ptr(),
                    marker.len() as u32,
                    coordinate,
                    &mut written,
                )
            },
            0
        );
        assert_eq!(written as usize, marker.len());
        let screen_restored = || {
            let mut actual = vec![0; marker.len()];
            let mut read = 0;
            assert_ne!(
                unsafe {
                    ReadConsoleOutputCharacterW(
                        output,
                        actual.as_mut_ptr(),
                        actual.len() as u32,
                        coordinate,
                        &mut read,
                    )
                },
                0
            );
            assert_eq!(read as usize, marker.len());
            assert_eq!(
                actual, marker,
                "the original screen returns after leaving the alternate screen"
            );
        };
        {
            let _guard = super::TerminalGuard::enter(true).unwrap();
            assert_ne!(mode() & ENABLE_VIRTUAL_TERMINAL_INPUT, 0);
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_time()
                .build()
                .unwrap();
            let mut events = runyte::tui::windows_input::EventStream::new().unwrap();
            let records: Vec<_> = "\x1b[200~:quit!\r\ncafé 😀\x1b[201~"
                .encode_utf16()
                .map(|unit| INPUT_RECORD {
                    EventType: KEY_EVENT as u16,
                    Event: INPUT_RECORD_0 {
                        KeyEvent: KEY_EVENT_RECORD {
                            bKeyDown: 1,
                            wRepeatCount: 1,
                            wVirtualKeyCode: 0,
                            wVirtualScanCode: 0,
                            uChar: KEY_EVENT_RECORD_0 { UnicodeChar: unit },
                            dwControlKeyState: 0,
                        },
                    },
                })
                .collect();
            let mut written = 0;
            assert_ne!(
                unsafe {
                    WriteConsoleInputW(input, records.as_ptr(), records.len() as u32, &mut written)
                },
                0
            );
            assert_eq!(written as usize, records.len());
            runtime.block_on(async {
                loop {
                    let event = tokio::time::timeout(Duration::from_secs(2), events.next())
                        .await
                        .unwrap()
                        .unwrap()
                        .unwrap();
                    if matches!(
                        event,
                        crossterm::event::Event::Resize(_, _)
                            | crossterm::event::Event::FocusGained
                            | crossterm::event::Event::FocusLost
                    ) {
                        continue;
                    }
                    assert_eq!(
                        event,
                        crossterm::event::Event::Paste(":quit!\r\ncafé 😀".into())
                    );
                    break;
                }
            });
            let repeated = INPUT_RECORD {
                EventType: KEY_EVENT as u16,
                Event: INPUT_RECORD_0 {
                    KeyEvent: KEY_EVENT_RECORD {
                        bKeyDown: 1,
                        wRepeatCount: 1024,
                        wVirtualKeyCode: 0,
                        wVirtualScanCode: 0,
                        uChar: KEY_EVENT_RECORD_0 {
                            UnicodeChar: b'x' as u16,
                        },
                        dwControlKeyState: 0,
                    },
                },
            };
            assert_ne!(
                unsafe { WriteConsoleInputW(input, &repeated, 1, &mut written) },
                0
            );
            runtime.block_on(async {
                tokio::time::timeout(Duration::from_secs(2), events.next())
                    .await
                    .unwrap()
                    .unwrap()
                    .unwrap();
            });
            let started = Instant::now();
            drop(events); // Cancels a reader even when the event channel fills.
            assert!(started.elapsed() < Duration::from_secs(2));
        }
        assert_eq!(mode(), original);
        screen_restored();
        let startup_failure: anyhow::Result<()> = (|| {
            let _guard = super::TerminalGuard::enter(true)?;
            assert_ne!(mode() & ENABLE_VIRTUAL_TERMINAL_INPUT, 0);
            anyhow::bail!("injected failure after terminal setup")
        })();
        assert!(
            startup_failure
                .unwrap_err()
                .to_string()
                .contains("injected failure")
        );
        assert_eq!(
            mode(),
            original,
            "startup failure restores the original input flags"
        );
        screen_restored();
    }
    #[cfg(unix)]
    use runyte::{
        selection::Selection, test_support::TestRuntimeRoot, text::Transaction,
        workspace::WorkspaceHost,
    };
    #[cfg(debug_assertions)]
    #[test]
    fn application_input_trace_redacts_keys_before_and_after_surface_closure() {
        let app = runyte::app::App::new(runyte::config::Config::default(), None).unwrap();
        let mut trace = Vec::new();
        for phase in ["before", "after"] {
            super::trace_input(
                Some(&mut trace),
                phase,
                &app,
                &runyte::input::InputEvent::Key(runyte::input::KeyStroke::char('🔑')),
                true,
                false,
                None,
            )
            .unwrap();
        }
        let text = String::from_utf8(trace).unwrap();
        assert!(!text.contains('🔑'));
        assert_eq!(text.matches("<application input redacted>").count(), 2);
    }

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
    #[cfg(not(windows))]
    use super::write_cwd_file;
    #[cfg(unix)]
    use super::{
        AttachedClient, AttachedWorkspaceActivity, PointerBatcher, apply_prepared_switch,
        atomic_write_cwd_file_with, dispatch_host_key_or_text, recover_switched_attachment,
        send_active_response, start_workspace_switch_host, unix_native_switch_refusal,
        unix_switch_selector,
    };
    use super::{
        KeyRepeatDetector, frame_publication_ready, initialize_attached_directory,
        is_passive_pointer, is_redraw_only_event, motion_repeat_dispatches,
        observe_key_or_text_hint, pace_file_picker_event, rejected_text_input,
        resolve_cwd_file_path, resolve_requested_project_root, starts_on_about,
        uses_automatic_persistent_mode, write_startup_screen,
    };
    #[cfg(windows)]
    use super::{convert_windows_event, is_empty_windows_image_paste, start_host_services};
    #[cfg(windows)]
    use crossterm::event::Event as CrosstermEvent;
    use runyte::launch::LaunchArguments;
    use runyte::{
        app::App,
        config::{Config, RunMode},
        input::{InputEvent, KeyCode, KeyStroke, Modifiers, PointerEvent, PointerEventKind},
        key_hints::KeyHintState,
        tui::input::convert_event,
    };

    #[cfg(windows)]
    #[tokio::test]
    async fn context_start_failure_precedes_other_service_ownership() {
        let root = runyte::test_support::TestRuntimeRoot::new("context-service-order").unwrap();
        let app = App::new_in_project(Config::default(), None, root.path()).unwrap();
        let mut host = runyte::workspace::WorkspaceHost::new(app);
        host.shutdown_context().await.unwrap();

        let error = start_host_services(
            &mut host,
            &mut runyte::startup::StartupTrace::new(),
            None,
            false,
            None,
        )
        .err()
        .expect("repeated context startup must fail");

        assert!(error.to_string().contains("shut down"));
        assert!(
            host.take_terminal_events().is_some(),
            "later service startup must not claim the terminal event owner"
        );
    }

    #[test]
    fn finder_refill_defers_unrelated_frame_requests_until_it_is_whole() {
        let mut frame_pending = false;

        assert!(
            !frame_publication_ready(true, true, &mut frame_pending),
            "an event arriving during a refill must not publish its partial list"
        );
        assert!(
            frame_pending,
            "the skipped request must survive until the refill completes"
        );
        assert!(frame_publication_ready(true, false, &mut frame_pending));

        frame_pending = false;
        assert!(!frame_publication_ready(false, true, &mut frame_pending));
        assert!(
            !frame_pending,
            "a refill with no frame request must not invent one"
        );
    }

    #[cfg(unix)]
    #[test]
    fn unix_refuses_provisional_native_switch_receipts() {
        for request in [
            runyte::protocol::ClientRequest::NativeSwitchCommit { receipt: 7 },
            runyte::protocol::ClientRequest::NativeParentSwitchCommitObserved { receipt: 7 },
            runyte::protocol::ClientRequest::NativeSwitchAbort { receipt: 7 },
        ] {
            assert!(matches!(
                unix_native_switch_refusal(&request),
                Some(runyte::protocol::HostResponse::Refused { message })
                    if message.contains("provisional native session switching")
            ));
        }
        assert!(unix_native_switch_refusal(&runyte::protocol::ClientRequest::Detach).is_none());
    }

    #[test]
    fn intermediate_file_picker_work_is_frame_paced_but_completion_is_immediate() {
        assert!(pace_file_picker_event(
            &runyte::file_picker::FilePickerEvent::Files {
                scan_id: 1,
                paths: Vec::new(),
            }
        ));
        assert!(!pace_file_picker_event(
            &runyte::file_picker::FilePickerEvent::Finished {
                scan_id: 1,
                skipped: 0,
                limited: false,
            }
        ));
    }

    #[cfg(unix)]
    #[test]
    fn terminal_loss_watcher_observes_pty_peer_close() {
        use std::{
            io::Write,
            os::fd::{AsRawFd, FromRawFd},
            sync::mpsc,
        };

        let mut master = -1;
        let mut slave = -1;
        // SAFETY: `openpty` initializes both descriptors on success. Null
        // termios and window-size pointers request the platform defaults.
        assert_eq!(
            unsafe {
                libc::openpty(
                    &mut master,
                    &mut slave,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                )
            },
            0,
            "openpty failed: {}",
            std::io::Error::last_os_error()
        );
        // SAFETY: successful `openpty` returned two fresh owned descriptors.
        let master = unsafe { std::fs::File::from_raw_fd(master) };
        // SAFETY: successful `openpty` returned two fresh owned descriptors.
        let slave = unsafe { std::fs::File::from_raw_fd(slave) };
        let (mut cancel, cancel_reader) = std::os::unix::net::UnixStream::pair().unwrap();
        let (sender, receiver) = mpsc::channel();
        let watcher = std::thread::spawn(move || {
            let result =
                super::wait_for_terminal_loss(slave.as_raw_fd(), cancel_reader.as_raw_fd());
            sender.send(result).unwrap();
        });

        drop(master);
        let result = match receiver.recv_timeout(Duration::from_secs(2)) {
            Ok(result) => result,
            Err(error) => {
                cancel.write_all(&[0]).unwrap();
                watcher.join().unwrap();
                panic!("terminal-loss watcher did not observe PTY close: {error}");
            }
        };
        watcher.join().unwrap();
        assert!(matches!(result, Some(Ok(()))));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn interrupted_process_queue_observation_keeps_note_exit() {
        let pid = 42;
        let mut observations = 0;
        let exited = super::process_queue_has_exited_with(pid, |event| {
            observations += 1;
            if observations == 1 {
                return Err(std::io::Error::from_raw_os_error(libc::EINTR));
            }
            event.write(libc::kevent {
                ident: pid as libc::uintptr_t,
                filter: libc::EVFILT_PROC,
                flags: 0,
                fflags: libc::NOTE_EXIT,
                data: 0,
                udata: std::ptr::null_mut(),
            });
            Ok(1)
        })
        .unwrap();
        assert!(exited);
        assert_eq!(observations, 2);
    }

    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn host_supervisor_process_queue_reports_child_exit() {
        let mut child = std::process::Command::new("sleep")
            .arg("120")
            .spawn()
            .unwrap();
        let supervisor = super::HostSupervisor::new(
            super::HostSupervisorKind::TestProcess,
            child.id() as libc::pid_t,
        )
        .unwrap();

        for _ in 0..32 {
            assert!(!supervisor.exited().unwrap());
            tokio::task::yield_now().await;
        }
        let unrelated = std::process::Command::new("true").status().unwrap();
        assert!(unrelated.success());
        assert!(!supervisor.exited().unwrap());

        child.kill().unwrap();
        let _ = child.wait().unwrap();
        tokio::time::timeout(Duration::from_secs(5), supervisor.recv())
            .await
            .expect("the process queue did not become readable")
            .unwrap();
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn host_supervisor_pidfd_reports_child_exit() {
        struct ChildGuard(std::process::Child);

        impl Drop for ChildGuard {
            fn drop(&mut self) {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }

        let Ok(child) = std::process::Command::new("sleep").arg("120").spawn() else {
            return;
        };
        let mut child = ChildGuard(child);
        let supervisor = super::HostSupervisor::new(
            super::HostSupervisorKind::TestProcess,
            child.0.id() as libc::pid_t,
        )
        .unwrap();
        if supervisor.pidfd.is_none() {
            return;
        }

        assert_eq!(supervisor.pid(), child.0.id() as libc::pid_t);
        assert!(!supervisor.exited().unwrap());
        child.0.kill().unwrap();
        let _ = child.0.wait().unwrap();
        tokio::time::timeout(Duration::from_secs(5), supervisor.recv())
            .await
            .expect("the pidfd did not become readable")
            .unwrap();
        assert!(supervisor.exited().unwrap());
    }

    #[cfg(unix)]
    #[test]
    fn process_termination_errors_keep_their_stable_user_messages() {
        assert_eq!(
            super::TerminatedBySignal(libc::SIGTERM).to_string(),
            format!("terminated by signal {}", libc::SIGTERM)
        );
        assert_eq!(
            super::terminated(libc::SIGINT).to_string(),
            format!("terminated by signal {}", libc::SIGINT)
        );
        #[cfg(unix)]
        assert_eq!(
            super::WaitTerminalLost.to_string(),
            "wait request lost its terminal before completion"
        );
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

        let root = TestRuntimeRoot::new("switch").unwrap();
        let source_root = root.join("source");
        let destination_root = root.join("destination");
        fs::create_dir_all(&source_root).unwrap();
        fs::create_dir_all(&destination_root).unwrap();
        let source = LocalEndpoint::discover_with_runtime(
            &source_root.join(".runyte"),
            &source_root,
            Some(root.path()),
        )
        .unwrap();
        let mut current = LocalEndpoint::discover_with_runtime(
            &destination_root.join(".runyte"),
            &destination_root,
            Some(root.path()),
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

        drop(root);
    }

    #[cfg(unix)]
    #[test]
    fn unix_switcher_rejects_native_keys_before_decoding_the_project_path() {
        let error = unix_switch_selector(
            runyte::protocol::WorkspaceSwitchTarget::Selected {
                project_root_bytes: vec![0xff; runyte::protocol::MAX_PATH_BYTES + 1],
                publication_key: Some([9; 32]),
            },
            None,
        )
        .unwrap_err();
        assert!(error.to_string().contains("native publication selections"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn replaced_executable_switch_failure_explains_how_to_recover() {
        use runyte::workspace::lifecycle::HostStartup;
        use runyte::workspace::transport::LocalEndpoint;

        let root = TestRuntimeRoot::new("switchxe").unwrap();
        let source_root = root.join("source");
        let destination_root = root.join("destination");
        fs::create_dir_all(&source_root).unwrap();
        fs::create_dir_all(&destination_root).unwrap();
        let source = LocalEndpoint::new(&source_root.join(".runyte"), &source_root).unwrap();
        let destination =
            LocalEndpoint::new(&destination_root.join(".runyte"), &destination_root).unwrap();
        let missing = root.join("replaced-runyte");

        let error =
            start_workspace_switch_host(&destination, HostStartup::new(&missing, "destination"))
                .await
                .unwrap_err();
        let message = format!("{error:#}");
        assert!(message.contains("detach with :detach"), "{message}");
        assert!(message.contains("launch Runyte again"), "{message}");
        assert!(message.contains("rebuilt, moved, or upgraded"), "{message}");

        let mut current = source;
        let mut previous = None;
        let mut notice = None;
        apply_prepared_switch(Err(error), &mut current, &mut previous, &mut notice);
        assert_eq!(current.project_root(), source_root);
        assert!(previous.is_none());
        assert_eq!(notice.as_deref(), Some(message.as_str()));

        drop(root);
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

    #[test]
    fn startup_screen_is_a_complete_document_free_presentation() {
        let mut output = Vec::new();

        write_startup_screen(&mut output).unwrap();

        let output = String::from_utf8(output).unwrap();
        assert!(output.contains("Runyte"));
        assert!(output.contains("Opening workspace…"));
        assert!(
            output.contains("\u{1b}[2J"),
            "startup screen was not cleared"
        );
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

    #[test]
    fn frontend_observes_the_configured_window_prefix_in_insert_mode() {
        let config = Config {
            keys: Some(serde_yaml::from_str("window: Ctrl-Q\n").unwrap()),
            ..Config::default()
        };
        let mut app = App::new(config, None).unwrap();
        app.mode = runyte::command::Mode::Insert;
        let mut hints = KeyHintState::default();
        let key = KeyStroke::new(KeyCode::Char('Q'), Modifiers::CONTROL | Modifiers::SHIFT);

        assert_eq!(
            observe_key_or_text_hint(&app, &mut hints, &InputEvent::Key(key)),
            runyte::key_hints::HintEventResult::Forward
        );
        assert_eq!(hints.pending().to_string(), "Ctrl-Q");
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

    #[cfg(windows)]
    #[test]
    fn empty_windows_image_paste_becomes_a_semantic_event_only_in_editor_panes() {
        let mut app = App::new(Config::default(), None).unwrap();
        let empty = CrosstermEvent::Paste(String::new());
        assert!(is_empty_windows_image_paste(&empty, &app));
        assert_eq!(
            convert_windows_event(empty.clone(), &app).unwrap(),
            Some(InputEvent::ClipboardPaste)
        );

        app.mode = runyte::command::Mode::Command;
        assert!(!is_empty_windows_image_paste(&empty, &app));
        app.mode = runyte::command::Mode::Normal;
        let text = CrosstermEvent::Paste("ordinary text".to_owned());
        assert!(!is_empty_windows_image_paste(&text, &app));

        for key in [' ', 'b', 'b'] {
            app.handle_input(InputEvent::Key(KeyStroke::char(key)))
                .unwrap();
        }
        assert!(app.has_input_overlay());
        assert!(!is_empty_windows_image_paste(&empty, &app));
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
    #[cfg(not(windows))]
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
        if cfg!(unix) {
            expected.push(0);
        }
        assert_eq!(fs::read(&first).unwrap(), expected);

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    #[cfg(windows)]
    fn relative_cwd_file_keeps_the_invoking_shells_identity_after_directory_changes() {
        let root = runyte::test_support::TestRuntimeRoot::new("cwd-relative").unwrap();
        let invoking = root.create_private_dir("shell").unwrap();
        let destination = root.create_private_dir("destination").unwrap();
        let first = resolve_cwd_file_path(&invoking, PathBuf::from("cwd"));
        let handoff = runyte::cwd_handoff::Prepared::prepare(&first).unwrap();
        let forwarded = resolve_cwd_file_path(&destination, first.clone());
        assert_eq!(forwarded, first);
        handoff.write(&destination).unwrap();
        assert!(first.is_file());
        assert!(!destination.join("cwd").exists());
        assert!(fs::read(&first).unwrap().starts_with(b"RNYCWD\x01\0"));
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
    fn targetless_launches_open_about_but_paths_keep_their_meaning() {
        let bare = LaunchArguments::parse_from([]).unwrap();
        let explicit_standalone = LaunchArguments::parse_from(["--ide".into()]).unwrap();
        let directory = LaunchArguments::parse_from([".".into()]).unwrap();
        let file = LaunchArguments::parse_from(["file.txt".into()]).unwrap();
        let server = LaunchArguments::parse_from(["--serve".into()]).unwrap();

        assert!(starts_on_about(&bare));
        assert!(starts_on_about(&explicit_standalone));
        assert!(!starts_on_about(&directory));
        assert!(!starts_on_about(&file));
        // A host is started without targets for an attaching client, so it
        // begins on the same page a bare standalone launch does.
        assert!(starts_on_about(&server));
    }

    #[test]
    fn persistent_default_only_changes_bare_implicit_launches() {
        let bare = LaunchArguments::parse_from([]).unwrap();
        let file = LaunchArguments::parse_from(["note.txt".into()]).unwrap();
        let directory = LaunchArguments::parse_from([".".into()]).unwrap();
        let positioned = LaunchArguments::parse_from(["+4:2".into(), "note.txt".into()]).unwrap();
        let explicit_standalone = LaunchArguments::parse_from(["--ide".into()]).unwrap();
        let init = LaunchArguments::parse_from(["--init".into(), "project".into()]).unwrap();

        assert!(uses_automatic_persistent_mode(&bare, RunMode::Mux));
        assert!(!uses_automatic_persistent_mode(&file, RunMode::Mux));
        assert!(!uses_automatic_persistent_mode(&directory, RunMode::Mux));
        assert!(!uses_automatic_persistent_mode(&positioned, RunMode::Mux));
        assert!(!uses_automatic_persistent_mode(
            &explicit_standalone,
            RunMode::Mux
        ));
        assert!(!uses_automatic_persistent_mode(&init, RunMode::Mux));
        assert!(!uses_automatic_persistent_mode(&bare, RunMode::Ide));
    }

    #[test]
    fn editor_mode_comes_from_the_flag_or_the_configured_default() {
        let parse = |arguments: &[&str]| {
            LaunchArguments::parse_from(arguments.iter().map(|argument| (*argument).into()))
                .unwrap()
        };
        let is_editor =
            |arguments: &[&str], configured| super::launch_is_editor(&parse(arguments), configured);
        assert!(is_editor(&["--editor", "/etc/fstab"], RunMode::Ide));
        assert!(is_editor(&["--editor"], RunMode::Mux));
        // The launch directory never decides: a target alone takes the
        // configured mode, which is ide unless set otherwise.
        assert!(!is_editor(&["/etc/fstab"], RunMode::Ide));
        assert!(is_editor(&["/etc/fstab"], RunMode::Editor));
        assert!(is_editor(&[], RunMode::Editor));
        // A mode option overrides the configured default.
        assert!(!is_editor(&["--ide", "/etc/fstab"], RunMode::Editor));
        assert!(!is_editor(&["--mux"], RunMode::Editor));
        assert!(!is_editor(&["--wait", "note.txt"], RunMode::Editor));
        // Options that name a workspace are not overridden by the default.
        assert!(!is_editor(
            &["--project-root", "/work", "note.txt"],
            RunMode::Editor
        ));
        assert!(!is_editor(&["--init", "/work/new"], RunMode::Editor));
    }

    /// An editor-mode host starts none of the services that assume a workspace.
    /// Nothing here reaches per-user storage: the services that would are
    /// exactly the ones that must stay off.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_plain_host_starts_no_workspace_service() {
        let root = TestRuntimeRoot::new("plain-host-services").unwrap();
        let mut app = App::new_in_project(Config::default(), None, root.path()).unwrap();
        app.enter_editor_mode();
        let mut host = WorkspaceHost::new(app);

        let mut services = super::start_host_services(
            &mut host,
            &mut runyte::startup::StartupTrace::new(),
            None,
            false,
        )
        .unwrap();

        assert_eq!(
            host.workspace_services_started(),
            runyte::workspace::WorkspaceServicesStarted::default()
        );
        assert!(services.git_events.is_none());
        assert!(services.plugin_events.is_none());
        assert!(services.workspace_events.is_none());
        assert!(
            services.context_events.recv().await.is_none(),
            "the context channel is closed rather than served"
        );
        assert!(!root.path().join(".runyte").exists());
        services
            .language_servers
            .send(runyte::lsp::LspCommand::Shutdown);
    }

    #[cfg(unix)]
    #[test]
    fn init_reports_a_new_and_an_existing_workspace() {
        let root = TestRuntimeRoot::new("init-report").unwrap();
        let project = root.path().join("project");
        std::fs::create_dir(&project).unwrap();
        let canonical = project.canonicalize().unwrap();
        let state = std::path::Path::new(".runyte");

        let mut output = Vec::new();
        super::initialize_workspace(
            std::path::Path::new("project"),
            root.path(),
            state,
            &[],
            &mut output,
        )
        .unwrap();
        assert_eq!(
            String::from_utf8(output).unwrap(),
            format!(
                "initialized a workspace in {0}\nopen it with: runyte {0}   or   runyte --mux {0}\n",
                canonical.display()
            )
        );
        assert!(canonical.join(".runyte").is_dir());

        let mut output = Vec::new();
        super::initialize_workspace(&canonical, root.path(), state, &[], &mut output).unwrap();
        assert!(
            String::from_utf8(output)
                .unwrap()
                .starts_with(&format!("{} is already a workspace\n", canonical.display()))
        );

        // Per-user storage stays protected, as it was when --init also opened
        // the editor.
        let reserved = root.path().join("config");
        let mut output = Vec::new();
        assert!(
            super::initialize_workspace(
                &canonical,
                root.path(),
                &reserved.join("state"),
                std::slice::from_ref(&reserved),
                &mut output,
            )
            .is_err()
        );
        assert!(output.is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn a_bare_parent_attachment_uses_the_existing_workspace_and_creates_none() {
        let root = TestRuntimeRoot::new("parent-selector").unwrap();
        let state = std::path::Path::new(".runyte");
        let plain = root.path().join("plain");
        std::fs::create_dir(&plain).unwrap();

        let error = super::parent_attach_selector(None, &plain, state).unwrap_err();
        assert!(error.to_string().contains("no workspace here"), "{error}");
        assert!(!plain.join(".runyte").exists());

        let project = root.path().join("project");
        std::fs::create_dir_all(project.join(".runyte")).unwrap();
        std::fs::create_dir(project.join("src")).unwrap();
        assert_eq!(
            super::parent_attach_selector(None, &project.join("src"), state).unwrap(),
            project.canonicalize().unwrap()
        );

        // A named selector is the caller's decision and passes unchanged.
        assert_eq!(
            super::parent_attach_selector(Some(&plain), &project, state).unwrap(),
            plain
        );
    }

    #[test]
    fn root_runs_only_editor_mode() {
        let parse = |arguments: &[&str]| {
            LaunchArguments::parse_from(arguments.iter().map(|argument| (*argument).into()))
                .unwrap()
        };
        for arguments in [
            &["/etc/hosts"][..],
            &["--ide", "/etc/hosts"][..],
            &["--mux"][..],
            &["--wait", "/etc/hosts"][..],
            &["--serve"][..],
            &["--init", "/srv/project"][..],
        ] {
            let arguments = parse(arguments);
            let editor = super::launch_is_editor(&arguments, RunMode::Ide);
            let error = super::refuse_workspace_modes_as_root(&arguments, editor, true)
                .expect_err("a workspace mode is refused as root");
            assert!(
                error.to_string().contains("runed"),
                "{arguments:?}: {error}"
            );
            assert!(
                super::refuse_workspace_modes_as_root(&arguments, editor, false).is_ok(),
                "{arguments:?}"
            );
        }
        for arguments in [&["--editor", "/etc/hosts"][..], &["--session-list"][..]] {
            let arguments = parse(arguments);
            let editor = super::launch_is_editor(&arguments, RunMode::Ide);
            assert!(
                super::refuse_workspace_modes_as_root(&arguments, editor, true).is_ok(),
                "{arguments:?}"
            );
        }
        // A configured editor default is editor mode as root too.
        let target = parse(&["/etc/hosts"]);
        let editor = super::launch_is_editor(&target, RunMode::Editor);
        assert!(super::refuse_workspace_modes_as_root(&target, editor, true).is_ok());
    }

    #[test]
    fn editor_keeps_a_bare_launch_off_the_persistent_default() {
        let editor = LaunchArguments::parse_from(["--editor".into()]).unwrap();
        assert!(!uses_automatic_persistent_mode(&editor, RunMode::Mux));
    }

    #[test]
    #[cfg(unix)]
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
        if cfg!(unix) {
            expected.push(0);
        }
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
    #[cfg(unix)]
    #[test]
    fn successful_attachment_history_ignores_preparation_and_repeated_attachments() {
        let source = runyte::workspace::transport::LocalEndpoint::new(
            Path::new("/tmp/runyte-history-source-state"),
            Path::new("/tmp/runyte-history-source"),
        )
        .unwrap();
        let target = runyte::workspace::transport::LocalEndpoint::new(
            Path::new("/tmp/runyte-history-target-state"),
            Path::new("/tmp/runyte-history-target"),
        )
        .unwrap();
        let mut history = super::AttachmentHistory::default();
        assert_eq!(history.attached(&source), None);
        let mut current = source.clone();
        let mut recovery = None;
        let mut notice = None;
        super::apply_prepared_switch(
            Ok(Some(target.clone())),
            &mut current,
            &mut recovery,
            &mut notice,
        );
        assert!(
            history.previous.is_none(),
            "preparing a destination is not a successful attachment"
        );
        assert_eq!(
            history.attached(&target),
            Some(source.project_root().to_owned())
        );
        assert_eq!(
            history.attached(&target),
            Some(source.project_root().to_owned())
        );
        assert_eq!(
            history.attached(&source),
            Some(target.project_root().to_owned())
        );
    }
}

#[cfg(all(test, unix, debug_assertions))]
#[path = "tui/tests/input_trace.rs"]
mod input_trace_tests;

#[cfg(all(test, any(unix, windows)))]
#[path = "tui/tests/session_table.rs"]
mod session_table_tests;
