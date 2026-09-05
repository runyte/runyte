// SPDX-License-Identifier: MPL-2.0

//! Bounded, process-owned diagnostic logging.
//!
//! The log is a durable chronology of lifecycle boundaries — process, host,
//! client, optional service, and panic — that survives a process failure. It
//! is not a second notification system and not an audit trail: an actionable
//! failure still reaches the person through the status line, `:notifications`,
//! and `:service-health` whether or not a record was written.
//!
//! Ownership follows editor-state ownership. The process that owns `App` owns
//! one file: a standalone editor writes `standalone-<pid>.log`, a persistent
//! host writes `host.log`. Default names cannot collide; on Unix, an explicit
//! path takes an advisory ownership lock so two processes cannot append or
//! rotate the same destination.
//!
//! Producers never wait for disk. A record is formatted, handed to a bounded
//! queue, and dropped if that queue is full; one background writer owns the
//! file and its rotation. Nothing in this module knows about buffers,
//! selections, panes, or protocol messages: instrumentation passes compact
//! values in at the boundaries that already own them.

use std::{
    fmt::{self, Display, Write as _},
    fs::{self, File},
    io::{Read, Seek, SeekFrom, Write},
    panic::PanicHookInfo,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, OnceLock, PoisonError,
        atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering},
        mpsc::{self, Receiver, SyncSender, TrySendError},
    },
    time::{Duration, Instant},
};

use chrono::{Local, SecondsFormat};

/// Bytes retained in the active file, and in the one previous file kept
/// beside it. Both bounds are applied to bytes rather than characters: record
/// text may embed arbitrary operating-system error strings.
pub const MAX_LOG_BYTES: u64 = 4 * 1024 * 1024;

/// How many previous files are kept beside the active one.
pub const RETAINED_PREVIOUS_FILES: usize = 1;

/// Records the queue holds before producers begin dropping them. Deep enough
/// that an ordinary burst of lifecycle events survives a slow disk, small
/// enough that a stalled writer cannot pin unbounded memory.
const QUEUE_CAPACITY: usize = 4096;

/// Lines of a captured panic backtrace that reach the log. A panicking
/// process is being wound down; the first frames identify it.
const MAX_BACKTRACE_LINES: usize = 64;

/// The canonical file name a persistent host owns.
pub const HOST_LOG_NAME: &str = "host.log";

/// Prefix of the per-process file a standalone editor owns.
pub const STANDALONE_LOG_PREFIX: &str = "standalone-";

/// How many standalone logs of processes that have exited are kept.
///
/// A crashed editor's log is the one somebody comes back to read, so stale
/// logs are not simply deleted; without a bound, though, every launch would
/// leave one behind forever and the workspace's diagnostic storage would not
/// actually be bounded. Retaining the newest few bounds stale history to this
/// many active/previous file pairs.
pub const RETAINED_STANDALONE_LOGS: usize = 4;

/// How long an explicit destination waits for a previous owner to release it.
///
/// A restart hands one path from an exiting process to its replacement, and
/// the old process still holds its log while it flushes and unwinds. Waiting
/// this long absorbs that handover; anything longer is a genuine second owner.
const OWNERSHIP_HANDOVER_BUDGET: Duration = Duration::from_secs(2);

/// How long a shutdown or panic flush waits before giving up. Both paths are
/// best effort and must never wait indefinitely.
pub const FLUSH_BUDGET: Duration = Duration::from_millis(500);

/// Retention levels, from the quietest to the most detailed.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum Level {
    Error,
    Warn,
    Info,
    Debug,
    Trace,
}

impl Level {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Error => "ERROR",
            Self::Warn => "WARN",
            Self::Info => "INFO",
            Self::Debug => "DEBUG",
            Self::Trace => "TRACE",
        }
    }

    /// The startup default: warnings and errors are retained without asking
    /// anybody to reproduce an unexpected first failure, and routine operation
    /// produces no high-volume trace.
    pub const fn default_level() -> Self {
        Self::Warn
    }

    /// Maps repeated `-v` occurrences onto levels, capped at trace.
    pub const fn from_verbosity(occurrences: u8) -> Self {
        match occurrences {
            0 => Self::Warn,
            1 => Self::Info,
            2 => Self::Debug,
            _ => Self::Trace,
        }
    }

    /// A non-zero rank. Zero is reserved for "no logger installed".
    const fn rank(self) -> u8 {
        match self {
            Self::Error => 1,
            Self::Warn => 2,
            Self::Info => 3,
            Self::Debug => 4,
            Self::Trace => 5,
        }
    }
}

impl Display for Level {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.label())
    }
}

/// Which process owns the log being written.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Role {
    Standalone,
    Host,
}

impl Role {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Standalone => "standalone",
            Self::Host => "host",
        }
    }
}

impl Display for Role {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.label())
    }
}

/// The file this process owns, given the state root that bounds runtime state.
///
/// A host has one canonical name because exactly one host serves a workspace.
/// More than one standalone editor may open the same workspace at once, so a
/// standalone name carries the process ID that owns it: two of them can never
/// write or rotate the same file.
pub fn default_path(state_root: &Path, role: Role, pid: u32) -> PathBuf {
    match role {
        Role::Host => state_root.join(HOST_LOG_NAME),
        Role::Standalone => state_root.join(format!("{STANDALONE_LOG_PREFIX}{pid}.log")),
    }
}

/// The previous file kept beside `path` after a rotation.
pub fn previous_path(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(".1");
    PathBuf::from(name)
}

/// Identity every record carries.
#[derive(Clone, Debug)]
pub struct Settings {
    pub level: Level,
    pub role: Role,
    /// The stable workspace ID, preferred over repeating an absolute project
    /// path on every record.
    pub workspace: Option<String>,
    pub pid: u32,
}

impl Settings {
    pub fn new(level: Level, role: Role) -> Self {
        Self {
            level,
            role,
            workspace: None,
            pid: std::process::id(),
        }
    }

    pub fn with_workspace(mut self, workspace: Option<String>) -> Self {
        self.workspace = workspace;
        self
    }
}

/// Where a logger's records go.
pub enum Sink {
    /// A rotating file.
    ///
    /// `exclusive` asks for the ownership the rotation model assumes. A
    /// default path does not need it — a host has one per workspace and a
    /// standalone name carries its PID — but an explicit `--log` is a path
    /// somebody typed, and two processes given the same one would interleave
    /// records and rotate over each other's files. Set it there, where
    /// refusing is already the specified behaviour for a destination that
    /// cannot be honoured.
    File { path: PathBuf, exclusive: bool },
    /// An in-memory or otherwise caller-owned destination. Not rotated: the
    /// caller owns whatever bound applies to it.
    Writer(Box<dyn Write + Send>),
}

impl Sink {
    /// A default destination, unique to this process by construction.
    pub fn file(path: impl Into<PathBuf>) -> Self {
        Self::File {
            path: path.into(),
            exclusive: false,
        }
    }

    /// A destination named on the command line, which must not already be
    /// owned by another live process.
    pub fn exclusive_file(path: impl Into<PathBuf>) -> Self {
        Self::File {
            path: path.into(),
            exclusive: true,
        }
    }
}

/// What `:service-health` reports about logging.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Status {
    pub role: Role,
    pub level: Option<Level>,
    pub path: Option<PathBuf>,
    /// Initialization or write failure, if logging is degraded.
    pub failure: Option<String>,
}

enum Message {
    /// A formatted record, and the level and target it was formatted with, so
    /// the writer can compose a summary in the same shape.
    Record {
        line: String,
        prefix: Arc<RecordPrefix>,
    },
    Flush(SyncSender<()>),
}

/// The identity every record of one process shares.
struct RecordPrefix {
    role: Role,
    pid: u32,
    workspace: Option<String>,
}

impl RecordPrefix {
    fn compose(&self, level: Level, target: &str, message: &str) -> String {
        let mut line = String::with_capacity(message.len() + 64);
        let _ = write!(
            line,
            "{} {:<5} {}[{}]",
            Local::now().to_rfc3339_opts(SecondsFormat::Millis, false),
            level.label(),
            self.role.label(),
            self.pid,
        );
        if let Some(workspace) = &self.workspace {
            let _ = write!(line, " ws={workspace}");
        }
        let _ = write!(line, " {target}: ");
        append_sanitized(&mut line, message);
        line.push('\n');
        line
    }
}

/// One process's bounded queue and its single background writer.
///
/// Constructed directly by tests, which need a logger that owns no global
/// state; production code builds one and hands it to [`install`].
pub struct Logger {
    sender: SyncSender<Message>,
    dropped: Arc<AtomicU64>,
    failure: Arc<Mutex<Option<String>>>,
    prefix: Arc<RecordPrefix>,
    settings: Settings,
    path: Option<PathBuf>,
}

impl Logger {
    /// Starts a logger and its writer.
    ///
    /// A file destination is created, rotated if the file it inherits is
    /// already at the bound, and opened for appending before this returns, so
    /// an unusable destination is reported to the caller rather than
    /// discovered later by a background thread.
    pub fn start(settings: Settings, sink: Sink) -> Result<Self, String> {
        let (path, mut destination) = match sink {
            Sink::File { path, exclusive } => {
                let (file, directory) = open_log_file(&path, exclusive)?;
                let size = file.metadata().map(|metadata| metadata.len()).unwrap_or(0);
                (
                    Some(path.clone()),
                    Destination::File {
                        file,
                        directory,
                        path,
                        size,
                    },
                )
            }
            Sink::Writer(writer) => (None, Destination::Writer(writer)),
        };
        let (sender, receiver) = mpsc::sync_channel(QUEUE_CAPACITY);
        let dropped = Arc::new(AtomicU64::new(0));
        let failure = Arc::new(Mutex::new(None));
        let prefix = Arc::new(RecordPrefix {
            role: settings.role,
            pid: settings.pid,
            workspace: settings.workspace.clone(),
        });
        std::thread::Builder::new()
            .name("runyte-log".to_owned())
            .spawn({
                let dropped = Arc::clone(&dropped);
                let failure = Arc::clone(&failure);
                move || write_records(&receiver, &mut destination, &dropped, &failure)
            })
            .map_err(|error| format!("cannot start the diagnostic log writer: {error}"))?;
        Ok(Self {
            sender,
            dropped,
            failure,
            prefix,
            settings,
            path,
        })
    }

    pub const fn level(&self) -> Level {
        self.settings.level
    }

    pub const fn role(&self) -> Role {
        self.settings.role
    }

    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// Records the queue refused because it was full. Diagnostic logging is
    /// best effort under load; nothing waits for disk.
    pub fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }

    /// The first write failure the background writer observed, if any.
    pub fn failure(&self) -> Option<String> {
        lock(&self.failure).clone()
    }

    pub fn enabled(&self, level: Level) -> bool {
        level <= self.settings.level
    }

    /// Formats one record and hands it to the queue without blocking.
    pub fn emit(&self, level: Level, target: &str, message: &str) {
        if !self.enabled(level) {
            return;
        }
        let line = self.prefix.compose(level, target, message);
        let record = Message::Record {
            line,
            prefix: Arc::clone(&self.prefix),
        };
        match self.sender.try_send(record) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) => {
                self.dropped.fetch_add(1, Ordering::Relaxed);
            }
            Err(TrySendError::Disconnected(_)) => {
                // The writer thread is gone, so every later record is lost
                // too. Recording that as a failure is what stops
                // `:service-health` from reporting a log nothing is writing
                // as healthy.
                let mut failure = lock(&self.failure);
                if failure.is_none() {
                    *failure = Some("the diagnostic log writer stopped".to_owned());
                }
            }
        }
    }

    /// Waits, at most `budget`, for everything already queued to reach disk.
    pub fn flush(&self, budget: Duration) {
        let deadline = Instant::now() + budget;
        let (acknowledge, acknowledged) = mpsc::sync_channel(1);
        let mut pending = Message::Flush(acknowledge);
        loop {
            match self.sender.try_send(pending) {
                Ok(()) => break,
                Err(TrySendError::Full(returned)) => {
                    if Instant::now() >= deadline {
                        return;
                    }
                    pending = returned;
                    std::thread::sleep(Duration::from_millis(1));
                }
                Err(TrySendError::Disconnected(_)) => return,
            }
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        let _ = acknowledged.recv_timeout(remaining);
    }
}

impl Drop for Logger {
    fn drop(&mut self) {
        self.flush(FLUSH_BUDGET);
    }
}

enum Destination {
    File {
        file: File,
        path: PathBuf,
        size: u64,
        directory: crate::private_storage::Directory,
    },
    Writer(Box<dyn Write + Send>),
}

impl Destination {
    fn write(&mut self, line: &str) -> std::io::Result<()> {
        match self {
            Self::File {
                file,
                path,
                size,
                directory,
            } => {
                let bytes = line.as_bytes();
                if *size > 0 && *size + bytes.len() as u64 > MAX_LOG_BYTES {
                    rotate(file, path, directory)?;
                    *size = 0;
                }
                file.write_all(bytes)?;
                file.flush()?;
                *size += bytes.len() as u64;
                Ok(())
            }
            Self::Writer(writer) => {
                writer.write_all(line.as_bytes())?;
                writer.flush()
            }
        }
    }
}

fn write_records(
    receiver: &Receiver<Message>,
    destination: &mut Destination,
    dropped: &AtomicU64,
    failure: &Mutex<Option<String>>,
) {
    let mut reported_drops = 0;
    while let Ok(message) = receiver.recv() {
        match message {
            Message::Record { line, prefix } => {
                let observed = dropped.load(Ordering::Relaxed);
                if observed > reported_drops {
                    // Composed through the same prefix as every other record,
                    // so a summary is one ordinary line rather than a second
                    // record format nothing else produces.
                    let summary = prefix.compose(
                        Level::Warn,
                        "log",
                        &format!(
                            "dropped {} diagnostic record(s) while the writer was behind",
                            observed - reported_drops
                        ),
                    );
                    reported_drops = observed;
                    record_failure(failure, destination.write(&summary));
                }
                record_failure(failure, destination.write(&line));
            }
            Message::Flush(acknowledge) => {
                let _ = acknowledge.try_send(());
            }
        }
    }
}

fn record_failure(failure: &Mutex<Option<String>>, result: std::io::Result<()>) {
    let Err(error) = result else {
        return;
    };
    let mut held = lock(failure);
    if held.is_none() {
        *held = Some(format!("cannot write the diagnostic log: {error}"));
    }
}

fn open_log_file(
    path: &Path,
    exclusive: bool,
) -> Result<(File, crate::private_storage::Directory), String> {
    let open = || -> std::io::Result<_> {
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let directory = crate::private_storage::Directory::open(parent, false)?;
        let name = path
            .file_name()
            .ok_or_else(|| std::io::Error::other("log path has no file name"))?;
        let file = directory.append(name)?;
        Ok((file, directory))
    };
    let (mut file, directory) = open()
        .map_err(|error| format!("cannot open the diagnostic log {}: {error}", path.display()))?;
    if exclusive {
        claim_ownership(&file, path)?;
    }
    if file.metadata().map_err(|e| e.to_string())?.len() >= MAX_LOG_BYTES {
        rotate(&mut file, path, &directory).map_err(|error| {
            format!(
                "cannot rotate the diagnostic log {}: {error}",
                path.display()
            )
        })?;
    }
    Ok((file, directory))
}

/// Copies the held file into a private atomic backup, then clears the same
/// inode. Neither source nor destination is reopened through an ambient path,
/// and an explicit log keeps its ownership lock throughout rotation.
fn rotate(
    file: &mut File,
    path: &Path,
    directory: &crate::private_storage::Directory,
) -> std::io::Result<()> {
    file.flush()?;
    file.seek(SeekFrom::Start(0))?;
    let mut bytes = Vec::new();
    file.take(MAX_LOG_BYTES).read_to_end(&mut bytes)?;
    directory.atomic_write(previous_path(path).file_name().unwrap(), &bytes)?;
    file.set_len(0)
}

/// Takes the advisory exclusive lock that makes process-owned rotation true.
///
/// A restart hands one explicit path from an exiting process to its
/// replacement, and the old one holds its log until it has finished flushing,
/// so a single refused attempt would turn an ordinary handover into a startup
/// failure. This waits out that window and only then reports a second owner.
fn claim_ownership(file: &File, path: &Path) -> Result<(), String> {
    let deadline = Instant::now() + OWNERSHIP_HANDOVER_BUDGET;
    loop {
        match try_lock_exclusive(file) {
            Ok(true) => return Ok(()),
            Ok(false) => {
                if Instant::now() >= deadline {
                    return Err(format!(
                        "the diagnostic log {} is already owned by another running Runyte \
process; choose a different --log path",
                        path.display()
                    ));
                }
                std::thread::sleep(Duration::from_millis(25));
            }
            Err(error) => {
                return Err(format!(
                    "cannot claim the diagnostic log {}: {error}",
                    path.display()
                ));
            }
        }
    }
}

/// `Ok(false)` means another process holds the lock.
///
/// Advisory locking is a Unix facility here. On other platforms an explicit
/// destination is opened without one, so two processes given the same `--log`
/// path there still share it.
#[cfg(unix)]
fn try_lock_exclusive(file: &File) -> std::io::Result<bool> {
    use std::os::fd::AsRawFd;

    // SAFETY: `file` owns a live descriptor for the duration of this call.
    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
        return Ok(true);
    }
    let error = std::io::Error::last_os_error();
    match error.raw_os_error() {
        Some(code) if code == libc::EWOULDBLOCK || code == libc::EINTR => Ok(false),
        _ => Err(error),
    }
}

#[cfg(not(unix))]
fn try_lock_exclusive(_file: &File) -> std::io::Result<bool> {
    Ok(true)
}

/// Removes standalone logs left by processes that have exited, newest first,
/// and returns how many files it deleted.
///
/// A standalone name carries its owner's PID so two live editors cannot share
/// a file, which also means every launch leaves one behind. The newest few are
/// kept because a crashed editor's log is exactly what somebody comes back to
/// read; everything older goes, along with the previous file beside it.
///
/// A live owner is never touched. On Unix that is checked directly; elsewhere
/// the platform refuses to delete an open file, which has the same effect.
pub fn prune_standalone_logs(directory: &Path, own_pid: u32, retain: usize) -> usize {
    let Ok(storage) = crate::private_storage::Directory::open(directory, false) else {
        return 0;
    };
    let Ok(entries) = fs::read_dir(directory) else {
        return 0;
    };
    let mut stale = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(pid) = name
            .to_str()
            .and_then(|name| name.strip_prefix(STANDALONE_LOG_PREFIX))
            .and_then(|rest| rest.strip_suffix(".log"))
            .and_then(|pid| pid.parse::<u32>().ok())
        else {
            continue;
        };
        if pid == own_pid || process_is_live(pid) {
            continue;
        }
        let modified = entry
            .metadata()
            .and_then(|metadata| metadata.modified())
            .unwrap_or(std::time::UNIX_EPOCH);
        stale.push((modified, entry.path()));
    }
    stale.sort_by_key(|entry| std::cmp::Reverse(entry.0));
    let mut removed = 0;
    for (_, path) in stale.into_iter().skip(retain) {
        if storage.remove(path.file_name().unwrap()).is_ok() {
            removed += 1;
        }
        if storage
            .remove(previous_path(&path).file_name().unwrap())
            .is_ok()
        {
            removed += 1;
        }
    }
    removed
}

/// Whether a process ID still names a running process.
///
/// Only ever used to decide against deleting somebody else's live log, so a
/// platform that cannot answer says yes.
#[cfg(unix)]
fn process_is_live(pid: u32) -> bool {
    let Ok(pid) = i32::try_from(pid) else {
        return true;
    };
    // SAFETY: signal 0 performs the existence and permission checks without
    // delivering anything.
    if unsafe { libc::kill(pid, 0) } == 0 {
        return true;
    }
    std::io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH)
}

#[cfg(not(unix))]
fn process_is_live(_pid: u32) -> bool {
    true
}

/// Collapses a record onto one line.
///
/// Records are read a line at a time, so an embedded newline in an operating
/// system error string would otherwise split one event into two. Control
/// characters are replaced rather than removed so nothing silently changes
/// length.
fn append_sanitized(line: &mut String, message: &str) {
    for character in message.chars() {
        if character.is_control() {
            line.push(' ');
        } else {
            line.push(character);
        }
    }
}

/// Appends one structured key-value pair to a record message.
pub fn append_field(message: &mut String, key: &str, value: &dyn Display) {
    let _ = write!(message, " {key}={value}");
}

static LEVEL: AtomicU8 = AtomicU8::new(0);
static LOGGER: OnceLock<Logger> = OnceLock::new();
static INITIALIZATION: Mutex<Option<Status>> = Mutex::new(None);
/// Whether the failure in `status()` has already reached the person.
static FAILURE_REPORTED: AtomicBool = AtomicBool::new(false);

/// Installs the process-wide logger. Later calls are ignored: a process owns
/// exactly one log for its lifetime, and no runtime reconfiguration exists.
pub fn install(logger: Logger) {
    let level = logger.level();
    let status = Status {
        role: logger.role(),
        level: Some(level),
        path: logger.path().map(Path::to_path_buf),
        failure: None,
    };
    if LOGGER.set(logger).is_ok() {
        *lock(&INITIALIZATION) = Some(status);
        LEVEL.store(level.rank(), Ordering::Relaxed);
    }
}

/// Records that logging is degraded because its destination was unusable.
///
/// Editing, and a host's ability to serve, continue regardless; the failure
/// reaches `:service-health` and the person through the ordinary surfaces.
pub fn note_unavailable(role: Role, path: Option<PathBuf>, failure: String) {
    *lock(&INITIALIZATION) = Some(Status {
        role,
        level: None,
        path,
        failure: Some(failure),
    });
}

/// The installed logger's owner role, level, destination, and any failure.
pub fn status() -> Option<Status> {
    let mut status = lock(&INITIALIZATION).clone()?;
    if status.failure.is_none() {
        status.failure = LOGGER.get().and_then(Logger::failure);
    }
    Some(status)
}

/// Whether a record at `level` would be retained. Instrumentation checks this
/// before formatting, so a disabled level costs one atomic load.
pub fn enabled(level: Level) -> bool {
    LEVEL.load(Ordering::Relaxed) >= level.rank()
}

/// Hands one already-formatted message to the installed logger.
pub fn emit(level: Level, target: &str, message: &str) {
    if let Some(logger) = LOGGER.get() {
        logger.emit(level, target, message);
    }
}

/// Returns a logger failure the person has not been told about yet.
///
/// Startup failures are reported by the caller that saw them. A destination
/// that becomes unwritable later — a full disk, a changed permission, a
/// rotation that fails — is observed only by the background writer, and
/// without this the log would stop silently until somebody happened to open
/// `:service-health`. Returns each failure once, so an event loop can call it
/// on an ordinary tick.
pub fn unreported_failure() -> Option<String> {
    let failure = status()?.failure?;
    (!FAILURE_REPORTED.swap(true, Ordering::Relaxed)).then_some(failure)
}

/// Marks the current failure as already reported, for a caller that surfaced
/// it itself.
pub fn note_failure_reported() {
    FAILURE_REPORTED.store(true, Ordering::Relaxed);
}

/// Records dropped by the installed logger because its queue was full.
pub fn dropped_records() -> u64 {
    LOGGER.get().map_or(0, Logger::dropped)
}

/// Bounded best-effort flush of the installed logger.
pub fn flush(budget: Duration) {
    if let Some(logger) = LOGGER.get() {
        logger.flush(budget);
    }
}

/// Bounded best-effort flush at orderly shutdown.
///
/// The installed logger is never dropped — it lives as long as the process —
/// so shutdown is a flush rather than a teardown. The writer thread ends with
/// the process it belongs to.
pub fn shutdown() {
    flush(FLUSH_BUDGET);
}

/// Chains a hook that leaves a final record before the ordinary panic output.
///
/// The normal output still reaches stderr, which is what a foreground or
/// standalone process is read through. A detached host has no such
/// destination, and its log is the only place the thread, location, message,
/// and backtrace survive. Unwinding is unaffected: the previous hook runs
/// afterwards exactly as it would have.
pub fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info: &PanicHookInfo<'_>| {
        record_panic(info);
        previous(info);
    }));
}

fn record_panic(info: &PanicHookInfo<'_>) {
    if !enabled(Level::Error) {
        return;
    }
    let thread = std::thread::current();
    let name = thread.name().unwrap_or("unnamed").to_owned();
    let location = info.location().map_or_else(
        || "unknown location".to_owned(),
        |location| {
            format!(
                "{}:{}:{}",
                location.file(),
                location.line(),
                location.column()
            )
        },
    );
    let payload = info.payload();
    let message = payload
        .downcast_ref::<&str>()
        .map(|text| (*text).to_owned())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "Box<dyn Any>".to_owned());
    emit(
        Level::Error,
        "panic",
        &format!("thread {name} panicked at {location}: {message}"),
    );
    let backtrace = std::backtrace::Backtrace::capture();
    if backtrace.status() == std::backtrace::BacktraceStatus::Captured {
        for frame in backtrace.to_string().lines().take(MAX_BACKTRACE_LINES) {
            emit(Level::Error, "panic", frame);
        }
    }
    flush(FLUSH_BUDGET);
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Emits one record at `$level` when that level is retained.
///
/// Structured context follows the message after a `;`, as `"key" => value`
/// pairs. Only compact, stable identifiers belong there: workspace ID, service
/// name, request ID, buffer ID and revision, terminal session ID, or transport
/// connection role. Document content, typed text, terminal output, and
/// unrestricted subprocess output never do.
#[macro_export]
macro_rules! log_record {
    ($level:expr, $target:expr, $format:literal $(, $argument:expr)* $(,)? ; $($key:literal => $value:expr),+ $(,)?) => {{
        let level = $level;
        if $crate::log::enabled(level) {
            let mut message = format!($format $(, $argument)*);
            $( $crate::log::append_field(&mut message, $key, &$value); )+
            $crate::log::emit(level, $target, &message);
        }
    }};
    ($level:expr, $target:expr, $format:literal $(, $argument:expr)* $(,)?) => {{
        let level = $level;
        if $crate::log::enabled(level) {
            $crate::log::emit(level, $target, &format!($format $(, $argument)*));
        }
    }};
}

#[macro_export]
macro_rules! log_error {
    ($($arguments:tt)*) => { $crate::log_record!($crate::log::Level::Error, $($arguments)*) };
}

#[macro_export]
macro_rules! log_warn {
    ($($arguments:tt)*) => { $crate::log_record!($crate::log::Level::Warn, $($arguments)*) };
}

#[macro_export]
macro_rules! log_info {
    ($($arguments:tt)*) => { $crate::log_record!($crate::log::Level::Info, $($arguments)*) };
}

#[macro_export]
macro_rules! log_debug {
    ($($arguments:tt)*) => { $crate::log_record!($crate::log::Level::Debug, $($arguments)*) };
}

#[macro_export]
macro_rules! log_trace {
    ($($arguments:tt)*) => { $crate::log_record!($crate::log::Level::Trace, $($arguments)*) };
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        sync::atomic::AtomicBool,
        time::{SystemTime, UNIX_EPOCH},
    };

    /// A destination the test can read back, standing in for a file.
    #[derive(Clone, Default)]
    struct Collected(Arc<Mutex<Vec<u8>>>);

    impl Collected {
        fn text(&self) -> String {
            String::from_utf8_lossy(&lock(&self.0)).into_owned()
        }
    }

    impl Write for Collected {
        fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
            lock(&self.0).extend_from_slice(buffer);
            Ok(buffer.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    /// A destination that cannot keep up, so the queue behind it fills.
    struct Slow(Arc<AtomicBool>);

    impl Write for Slow {
        fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
            while !self.0.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_millis(5));
            }
            Ok(buffer.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    fn temporary(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "runyte-log-{name}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    fn collecting(level: Level) -> (Logger, Collected) {
        let collected = Collected::default();
        let logger = Logger::start(
            Settings::new(level, Role::Standalone).with_workspace(Some("a1b2c3d4".to_owned())),
            Sink::Writer(Box::new(collected.clone())),
        )
        .unwrap();
        (logger, collected)
    }

    #[test]
    fn repeated_verbosity_selects_the_documented_levels_and_caps_at_trace() {
        assert_eq!(Level::from_verbosity(0), Level::Warn);
        assert_eq!(Level::from_verbosity(0), Level::default_level());
        assert_eq!(Level::from_verbosity(1), Level::Info);
        assert_eq!(Level::from_verbosity(2), Level::Debug);
        assert_eq!(Level::from_verbosity(3), Level::Trace);
        for occurrences in 4..=u8::MAX {
            assert_eq!(Level::from_verbosity(occurrences), Level::Trace);
        }
    }

    #[test]
    fn the_default_level_retains_warnings_and_errors_only() {
        let (logger, collected) = collecting(Level::default_level());
        for level in [
            Level::Error,
            Level::Warn,
            Level::Info,
            Level::Debug,
            Level::Trace,
        ] {
            logger.emit(level, "test", &format!("record at {level}"));
        }
        logger.flush(FLUSH_BUDGET);

        let text = collected.text();
        assert!(text.contains("record at ERROR"), "{text}");
        assert!(text.contains("record at WARN"), "{text}");
        for absent in ["record at INFO", "record at DEBUG", "record at TRACE"] {
            assert!(!text.contains(absent), "{absent} reached the log:\n{text}");
        }
    }

    #[test]
    fn each_raised_level_admits_exactly_what_it_documents() {
        for (level, retained) in [(Level::Info, 3), (Level::Debug, 4), (Level::Trace, 5)] {
            let (logger, collected) = collecting(level);
            for emitted in [
                Level::Error,
                Level::Warn,
                Level::Info,
                Level::Debug,
                Level::Trace,
            ] {
                logger.emit(emitted, "test", "record");
            }
            logger.flush(FLUSH_BUDGET);
            assert_eq!(collected.text().lines().count(), retained, "at {level}");
        }
    }

    #[test]
    fn a_record_carries_its_role_process_workspace_and_target_on_one_line() {
        let (logger, collected) = collecting(Level::Warn);
        logger.emit(Level::Warn, "transport", "ended\ninside\ta message");
        logger.flush(FLUSH_BUDGET);

        let text = collected.text();
        assert_eq!(text.lines().count(), 1, "{text}");
        assert!(text.contains("WARN "), "{text}");
        assert!(
            text.contains(&format!("standalone[{}]", std::process::id())),
            "{text}"
        );
        assert!(text.contains("ws=a1b2c3d4"), "{text}");
        assert!(text.contains("transport: "), "{text}");
        assert!(text.contains("ended inside a message"), "{text}");
    }

    #[test]
    fn structured_context_is_appended_as_key_value_pairs() {
        let mut message = "language server stopped".to_owned();
        append_field(&mut message, "language", &"rust");
        append_field(&mut message, "generation", &7);
        assert_eq!(
            message,
            "language server stopped language=rust generation=7"
        );
    }

    #[test]
    fn one_workspace_cannot_have_two_standalone_processes_share_a_file() {
        let root = Path::new("/project/.runyte");
        assert_eq!(
            default_path(root, Role::Host, 11),
            root.join("host.log"),
            "a host owns one canonical file"
        );
        assert_eq!(
            default_path(root, Role::Host, 11),
            default_path(root, Role::Host, 12)
        );
        assert_ne!(
            default_path(root, Role::Standalone, 11),
            default_path(root, Role::Standalone, 12),
            "concurrent standalone editors must not share a writable log"
        );
        assert_eq!(
            default_path(root, Role::Standalone, 11),
            root.join("standalone-11.log")
        );
    }

    #[test]
    fn an_inherited_full_file_is_rotated_before_the_first_record() {
        let directory = temporary("startup-rotation");
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join("host.log");
        File::create(&path).unwrap().set_len(MAX_LOG_BYTES).unwrap();

        let logger = Logger::start(
            Settings::new(Level::Warn, Role::Host),
            Sink::file(path.clone()),
        )
        .unwrap();
        logger.emit(Level::Warn, "test", "after restart");
        logger.flush(FLUSH_BUDGET);

        assert_eq!(
            fs::metadata(previous_path(&path)).unwrap().len(),
            MAX_LOG_BYTES
        );
        let active = fs::read_to_string(&path).unwrap();
        assert!(active.contains("after restart"), "{active}");
        assert!(active.len() < 512, "the active file restarted empty");

        drop(logger);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn rotation_keeps_one_previous_file_and_never_a_second() {
        let directory = temporary("rotation-bound");
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join("host.log");
        let logger = Logger::start(
            Settings::new(Level::Warn, Role::Host),
            Sink::file(path.clone()),
        )
        .unwrap();

        // Half a mebibyte per record, so the four-mebibyte bound is crossed
        // twice in a couple of dozen writes rather than tens of thousands.
        let bulk = "x".repeat(512 * 1024);
        let rounds = (MAX_LOG_BYTES / (512 * 1024)) * 2 + 4;
        for round in 0..rounds {
            logger.emit(Level::Warn, "test", &format!("round {round} {bulk}"));
        }
        logger.flush(FLUSH_BUDGET);

        let mut names = fs::read_dir(&directory)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        names.sort();
        assert_eq!(
            names.len(),
            1 + RETAINED_PREVIOUS_FILES,
            "rotation keeps the active file and exactly one previous file"
        );
        assert_eq!(names, vec!["host.log".to_owned(), "host.log.1".to_owned()]);
        assert!(fs::metadata(&path).unwrap().len() <= MAX_LOG_BYTES);
        assert!(fs::metadata(previous_path(&path)).unwrap().len() <= MAX_LOG_BYTES);
        assert!(
            fs::read_to_string(&path)
                .unwrap()
                .contains(&format!("round {}", rounds - 1)),
            "the newest record stays in the active file"
        );
        assert_eq!(logger.dropped(), 0, "a keeping-up writer drops nothing");

        drop(logger);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn a_stalled_writer_drops_records_instead_of_stalling_its_producer() {
        let released = Arc::new(AtomicBool::new(false));
        let logger = Logger::start(
            Settings::new(Level::Trace, Role::Host),
            Sink::Writer(Box::new(Slow(Arc::clone(&released)))),
        )
        .unwrap();

        let started = Instant::now();
        for index in 0..QUEUE_CAPACITY * 8 {
            logger.emit(Level::Warn, "test", &format!("record {index}"));
        }
        let elapsed = started.elapsed();
        released.store(true, Ordering::Relaxed);

        assert!(
            elapsed < Duration::from_secs(2),
            "producers waited {elapsed:?} on a stalled writer"
        );
        assert!(
            logger.dropped() > 0,
            "a saturated queue must drop rather than block"
        );
    }

    #[test]
    fn an_unusable_destination_is_reported_rather_than_discovered_later() {
        let directory = temporary("unusable");
        fs::create_dir_all(&directory).unwrap();
        let occupied = directory.join("occupied");
        fs::write(&occupied, "not a directory").unwrap();

        let failure = match Logger::start(
            Settings::new(Level::Warn, Role::Standalone),
            Sink::file(occupied.join("host.log")),
        ) {
            Ok(_) => panic!("an unusable destination must be reported"),
            Err(failure) => failure,
        };
        assert!(failure.contains("diagnostic log"), "{failure}");

        fs::remove_dir_all(directory).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn stale_standalone_logs_are_bounded_without_touching_a_live_owner() {
        let directory = temporary("standalone-retention");
        fs::create_dir_all(&directory).unwrap();

        let mut candidate = 1_500_000_000_u32;
        let stale_pids = (0..6)
            .map(|_| {
                while process_is_live(candidate) {
                    candidate -= 1;
                }
                let pid = candidate;
                candidate -= 1;
                pid
            })
            .collect::<Vec<_>>();
        for (age, pid) in stale_pids.iter().enumerate() {
            let path = default_path(&directory, Role::Standalone, *pid);
            fs::write(&path, format!("stale {age}")).unwrap();
            fs::write(previous_path(&path), format!("previous {age}")).unwrap();
            let modified = UNIX_EPOCH + Duration::from_secs(age as u64 + 1);
            let times = fs::FileTimes::new().set_modified(modified);
            File::open(&path).unwrap().set_times(times).unwrap();
            File::open(previous_path(&path))
                .unwrap()
                .set_times(times)
                .unwrap();
        }

        let live = default_path(&directory, Role::Standalone, std::process::id());
        fs::write(&live, "live").unwrap();
        fs::write(previous_path(&live), "live previous").unwrap();
        let unrelated = directory.join("notes.log");
        fs::write(&unrelated, "not a standalone log").unwrap();

        let removed = prune_standalone_logs(&directory, candidate, RETAINED_STANDALONE_LOGS);

        assert_eq!(removed, 4, "two stale logs and their rotations are pruned");
        for pid in &stale_pids[..2] {
            let path = default_path(&directory, Role::Standalone, *pid);
            assert!(!path.exists(), "the oldest stale log survived: {path:?}");
            assert!(!previous_path(&path).exists());
        }
        for pid in &stale_pids[2..] {
            let path = default_path(&directory, Role::Standalone, *pid);
            assert!(path.exists(), "a retained stale log was removed: {path:?}");
            assert!(previous_path(&path).exists());
        }
        assert!(live.exists(), "a live process's active log was removed");
        assert!(
            previous_path(&live).exists(),
            "a live process's rotated log was removed"
        );
        assert!(unrelated.exists(), "an unrelated file was removed");

        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn a_logger_failure_is_reported_once() {
        *lock(&INITIALIZATION) = Some(Status {
            role: Role::Standalone,
            level: Some(Level::Warn),
            path: Some(PathBuf::from("diagnostic.log")),
            failure: Some("cannot write the diagnostic log: disk full".to_owned()),
        });
        FAILURE_REPORTED.store(false, Ordering::Relaxed);

        assert_eq!(
            unreported_failure().as_deref(),
            Some("cannot write the diagnostic log: disk full")
        );
        assert_eq!(unreported_failure(), None, "the same failure was repeated");
        note_failure_reported();
        assert_eq!(unreported_failure(), None);

        *lock(&INITIALIZATION) = None;
        FAILURE_REPORTED.store(false, Ordering::Relaxed);
    }
}
