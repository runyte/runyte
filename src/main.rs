// SPDX-License-Identifier: MPL-2.0

use std::{
    fs,
    io::{self, Write, stdout},
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

#[cfg(unix)]
use std::thread;

use anyhow::{Context, Result};
#[cfg(unix)]
use crossterm::event::{
    KeyboardEnhancementFlags, PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
};
use crossterm::{
    ExecutableCommand, QueueableCommand,
    cursor::{Hide, MoveTo, Show},
    event::{
        DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
        Event as CrosstermEvent, EventStream, KeyEventKind,
    },
    style::{Attribute, Print, SetAttribute},
    terminal::{
        Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode,
        enable_raw_mode,
    },
};
use futures_util::StreamExt;
use ratatui::{Terminal, backend::CrosstermBackend};
use runyte::{
    app::{App, PersistentExitRequest},
    command::{
        CommandCategory, CommandExecutionContext, CommandInvocation, CommandInvocationError,
        EditorCommand,
    },
    config::{self, Config, WorkspaceMode},
    external_open, file_monitor, file_picker,
    git::{GitCliProvider, GitService, GitServiceEvent},
    git_monitor,
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
/// still: this bounds how often its terminal rows can change.
const FINDER_TERMINAL_REFRESH_INTERVAL: Duration = Duration::from_secs(3);
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
    LocalEndpoint, LocalServer, ServerEvent, TransportChange, decode_path, encode_path,
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

#[derive(Debug)]
struct TerminatedBySignal(i32);

impl std::fmt::Display for TerminatedBySignal {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "terminated by signal {}", self.0)
    }
}

impl std::error::Error for TerminatedBySignal {}

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

    let mut event = std::mem::MaybeUninit::<libc::kevent>::uninit();
    let timeout = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: the kqueue descriptor is live, the output has capacity for one
    // event, and the zero timeout performs a non-blocking observation.
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
        return Err(std::io::Error::last_os_error())
            .context("cannot read host supervisor process queue");
    }
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
    // Preserve and publish the ordinary terminal state before installing the
    // handler. A signal before installation keeps its safe default disposition;
    // every signal after installation sees startup protection already armed.
    let startup_restore = if arguments.mode == LaunchMode::Standalone {
        Some(StartupSignalExit::arm())
    } else {
        None
    };
    let mut standalone_termination = if startup_restore
        .as_ref()
        .is_some_and(std::result::Result::is_ok)
    {
        Some(TerminationSignals::new()?)
    } else {
        None
    };
    let mut startup_signal_exit = None;
    let standalone_terminal = if arguments.mode == LaunchMode::Standalone {
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

    // The standalone resources were acquired before editor construction. Move
    // them into the interactive loop now that the persistent-host branch has
    // returned.
    let color_depth = standalone_color_depth.expect("standalone terminal colour depth");
    let _terminal = standalone_terminal.expect("standalone terminal guard")?;
    let mut termination = standalone_termination
        .take()
        .expect("standalone termination signals");
    let backend = CrosstermBackend::new(stdout());
    let mut terminal = Terminal::new(backend)?;
    let mut received_signal = None;
    let mut key_hints = KeyHintState::default();
    if show_startup_about {
        app.app_mut().execute(about_invocation()?)?;
    }
    terminal.draw(|frame| {
        let geometry = ui::frame_geometry(frame.area());
        let snapshot = app.prepare_frame_with_hints(geometry, Some(&key_hints));
        ui::render(frame, app.app(), &snapshot.editor, &key_hints, color_depth);
    })?;
    startup.mark(StartupPhase::EditorFramePresented);
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
        ui::render(frame, app.app(), &snapshot.editor, &key_hints, color_depth);
    })?;
    drop(startup_signal_exit.take());
    let mut terminal_events = EventStream::new();
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
                            if frame_publication_ready(
                                true,
                                app.finder_scan_refills(),
                                &mut frame_pending,
                            ) {
                                terminal.draw(|frame| {
                                    let geometry = ui::frame_geometry(frame.area());
                                    let snapshot = app.prepare_frame_with_hints(
                                        geometry,
                                        Some(&key_hints),
                                    );
                                    ui::render(
                                        frame,
                                        app.app(),
                                        &snapshot.editor,
                                        &key_hints,
                                        color_depth,
                                    );
                                })?;
                                frame_pending = false;
                            }
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
            event = services.git_monitor_events.recv() => {
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
                services.git_monitor.sync(app.git_monitor_repository());
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
            _ = frame_tick.tick(), if frame_pending && !app.finder_scan_refills() => {}
            _ = tokio::time::sleep(hint_timeout.unwrap_or_default()), if hint_timeout.is_some() => {
                key_hints.expire_at(Instant::now());
            }
            signal = termination.recv() => {
                received_signal = Some(signal);
                break;
            }
        }
        if !frame_publication_ready(true, app.finder_scan_refills(), &mut frame_pending) {
            continue;
        }
        terminal.draw(|frame| {
            let geometry = ui::frame_geometry(frame.area());
            let snapshot = app.prepare_frame_with_hints(geometry, Some(&key_hints));
            ui::render(frame, app.app(), &snapshot.editor, &key_hints, color_depth);
        })?;
        frame_pending = false;
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
                                if frame_publication_ready(
                                    true,
                                    host.finder_scan_refills(),
                                    &mut frame_pending,
                                ) {
                                    publish_attached_frame(&mut host, &mut active, &key_hints);
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
            event = services.git_monitor_events.recv() => {
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
                    host.apply_terminal_output(output, observed);
                    terminal::drain(&mut services.terminal_events, |output| {
                        host.apply_terminal_output(output, observed);
                    });
                    frame_pending = true;
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
                services.git_monitor.sync(host.git_monitor_repository());
                changed = host.refresh_git_if_due(Instant::now());
                changed |= host.refresh_session_activity();
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
            _ = frame_tick.tick(), if frame_pending
                && active.is_some()
                && !host.finder_scan_refills() =>
            {
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
        if frame_publication_ready(changed, host.finder_scan_refills(), &mut frame_pending) {
            publish_attached_frame(&mut host, &mut active, &key_hints);
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

fn terminal_color_depth() -> ui::TerminalColorDepth {
    ui::TerminalColorDepth::from_color_count(crossterm::style::available_color_count())
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
    let color_depth = terminal_color_depth();
    let mut termination = TerminationSignals::new()?;
    let _terminal = TerminalGuard::enter(mouse_enabled)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout()))?;
    // Probe the terminal before the event stream exists. Where `TIOCGWINSZ` is
    // unavailable Crossterm falls back to asking the terminal for its cursor
    // position, and an event reader would consume the reply.
    let mut geometry = current_frame_geometry()?;
    let mut terminal_events = AttachedTerminalEvents::stream();
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
                AttachOptions {
                    wait_token: None,
                    cwd_file,
                    notice: notice.take(),
                    color_depth,
                },
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
                let prepared = prepare_switch_target(
                    &selector,
                    &working_directory,
                    &current,
                    config,
                    config_path,
                )
                .await;
                apply_prepared_switch(prepared, &mut current, &mut previous, &mut notice);
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
        Ok(AttachOutcome::Detached) => Ok(()),
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
enum AttachedTerminalEvents {
    Stream(EventStream),
    Isolated(tokio::sync::mpsc::UnboundedReceiver<io::Result<CrosstermEvent>>),
}

#[cfg(unix)]
impl AttachedTerminalEvents {
    fn stream() -> Self {
        Self::Stream(EventStream::new())
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
        Ok(Self::Isolated(receiver))
    }

    async fn next(&mut self) -> Option<io::Result<CrosstermEvent>> {
        match self {
            Self::Stream(stream) => stream.next().await,
            Self::Isolated(receiver) => receiver.recv().await,
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

/// Client-local state that changes how one host attachment is presented.
#[cfg(unix)]
struct AttachOptions<'a> {
    wait_token: Option<WaitToken>,
    cwd_file: Option<&'a Path>,
    notice: Option<String>,
    color_depth: ui::TerminalColorDepth,
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
    terminal_events: &mut AttachedTerminalEvents,
    geometry: &mut runyte::app::FrameGeometry,
    options: AttachOptions<'_>,
) -> Result<AttachOutcome> {
    let AttachOptions {
        wait_token,
        cwd_file,
        notice,
        color_depth,
    } = options;
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
    let mut client = client.buffer_responses();
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
                    terminal
                        .draw(|frame| ui::render_host_frame(frame, &current_frame, color_depth))?;
                }
                Some(HostResponse::TerminalDamage { damage }) => {
                    if apply_terminal_damage(&mut current_frame, &damage)? {
                        terminal.draw(|frame| {
                            ui::render_host_frame(frame, &current_frame, color_depth)
                        })?;
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
    terminal.draw(|frame| ui::render_host_frame(frame, &current_frame, color_depth))?;
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
                        terminal.draw(|frame| {
                            ui::render_host_frame(
                                frame,
                                &current_frame,
                                color_depth,
                            )
                        })?;
                    }
                    Some(HostResponse::TerminalDamage { damage }) => {
                        if apply_terminal_damage(&mut current_frame, &damage)? {
                            terminal.draw(|frame| {
                                ui::render_host_frame(
                                    frame,
                                    &current_frame,
                                    color_depth,
                                )
                            })?;
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
                    Some(HostResponse::ShuttingDown) => {
                        anyhow::ensure!(wait_token.is_none(), "wait request ended before completion");
                        break;
                    }
                    None => {
                        anyhow::bail!("workspace host disconnected without ending the attachment");
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
                Ok(Some(HostResponse::Frame { .. } | HostResponse::TerminalDamage { .. })) => {}
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

#[cfg(unix)]
async fn list_sessions(state: &Path, include_hidden: bool) -> Result<()> {
    let workspaces = if include_hidden {
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
async fn wait_for_completion(
    client: &mut LocalClient,
    endpoint: &LocalEndpoint,
    mouse_enabled: bool,
    token: WaitToken,
    termination: &mut TerminationSignals,
    terminal_loss: &mut TerminalLoss,
    launching_parent: &HostSupervisor,
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
            response = client.recv() => response?,
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

struct HostServices {
    syntax_events: SyntaxEvents,
    git_events: Option<tokio::sync::mpsc::Receiver<GitServiceEvent>>,
    language_servers: LspHandle,
    lsp_events: tokio::sync::mpsc::Receiver<LspEvent>,
    file_picker_events: tokio::sync::mpsc::Receiver<runyte::file_picker::FilePickerEvent>,
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
    let (mut file_monitor, file_monitor_events) = file_monitor::spawn();
    file_monitor.sync(app.file_monitor_requests());
    let (mut git_monitor, git_monitor_events) = git_monitor::spawn();
    git_monitor.sync(app.git_monitor_repository());
    app.attach_word_index(word_index::spawn());
    #[cfg(unix)]
    let workspace_events = {
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
        git_monitor,
        git_monitor_events,
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

/// Draws the stable presentation used while the first editor state is built.
///
/// This screen deliberately contains no document text: replacing it with the
/// first complete editor frame cannot flash incomplete syntax or reflow text.
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
        --init DIRECTORY Make DIRECTORY the exact standalone workspace root
                         and open it
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
                         if needed. If WORKSPACE is omitted, use the workspace
                         found from the current directory, or make that
                         directory a workspace when none is found

PERSISTENT SESSIONS:
    A persistent session is the durable local process and retained editor state
    associated with one workspace. CLI listing also works from standalone mode;
    session commands inside the editor need workspace.mode: persistent.

    WORKSPACE selects a session by ID, unambiguous ID prefix, persistent name,
    or directory, so a session is reachable from anywhere.

        --serve          Keep a persistent session alive in the foreground
        --wait FILE...   Edit files through persistent mode and wait for
                         explicit completion
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

    Naming a target always runs standalone, so its relative path and caret
    position keep their ordinary meaning: workspace.mode: persistent changes
    only a bare runyte, and --persistent reads its argument as a workspace
    rather than a file. Use --init to make a directory the exact standalone
    workspace root, or --wait to open files through a persistent session.

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
        WaitToken, apply_prepared_switch, atomic_write_cwd_file_with, dispatch_host_key_or_text,
        recover_switched_attachment, send_active_response, start_workspace_switch_host,
        workspace_response_publishes_frame,
    };
    use super::{
        KeyRepeatDetector, frame_publication_ready, initialize_attached_directory,
        is_passive_pointer, is_redraw_only_event, motion_repeat_dispatches,
        observe_key_or_text_hint, rejected_text_input, resolve_cwd_file_path,
        resolve_requested_project_root, starts_on_about, uses_automatic_persistent_mode,
        write_cwd_file, write_startup_screen,
    };
    use runyte::launch::LaunchArguments;
    use runyte::{
        app::App,
        config::{Config, WorkspaceMode},
        input::{InputEvent, KeyCode, KeyStroke, Modifiers, PointerEvent, PointerEventKind},
        key_hints::KeyHintState,
        selection::Selection,
        test_support::TestRuntimeRoot,
        text::Transaction,
        tui::input::convert_event,
        workspace::WorkspaceHost,
    };

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
    fn targetless_launches_open_about_but_paths_keep_their_meaning() {
        let bare = LaunchArguments::parse_from([]).unwrap();
        let explicit_standalone = LaunchArguments::parse_from(["--standalone".into()]).unwrap();
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
