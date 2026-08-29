// SPDX-License-Identifier: MPL-2.0

//! The Git command-line implementation of [`GitProvider`].
//!
//! Every call here is an argument vector handed to `git` directly. There is no
//! shell, so no path, branch, or filename can turn into syntax; arguments that
//! could be read as options are separated with `--`, and paths are passed as
//! `OsStr` so a name that is not UTF-8 survives the trip.
//!
//! Output is bounded before it is read. A repository can contain a blob larger
//! than the machine's memory, and the editor must decline that rather than try
//! to hold it: the child is killed and the call fails with
//! [`GitError::TooLarge`].

use std::{
    ffi::{OsStr, OsString},
    io::{self, Read, Write},
    path::{Component, Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use super::{
    BaseContent, BlameLine, BlameRequest, Branch, BranchDeletionPlan, CommitDetail,
    CommitSearchResult, DeletionAuthorization, DiffScope, Divergence, FileComparison, GitError,
    GitProvider, Head, LogCursor, LogPage, LogRequest, MAX_BLAME_INPUT_BYTES, MAX_BLAME_LINES,
    MAX_COMMIT_SEARCH_RESULTS, MAX_LOG_PAGE_SIZE, MAX_PATCH_BYTES, PartialStageRequest,
    PartialStageSelection, Repository, RepositoryFingerprint, RepositoryStatus, Result, StashEntry,
    StashMutation, StashScope, StatusStats, Upstream, Worktree, WorktreeCreate,
    WorktreeRemovalPlan, count_new_lines, history::valid_object_id, parse_blame,
    parse_commit_search, parse_log, parse_numstat, parse_stashes, parse_worktree_porcelain,
    patch::valid_fingerprint, stats::LineStats, status,
};

/// How much output one call will hold.
///
/// Large enough for the status of a very large repository and for the source
/// files people actually edit, small enough that a pathological blob cannot
/// take the editor down with it.
pub const DEFAULT_MAX_OUTPUT_BYTES: usize = 16 * 1024 * 1024;

/// How much output a single commit's patch may hold.
///
/// A commit that touches many large files can produce a unified diff well
/// past [`DEFAULT_MAX_OUTPUT_BYTES`] without being unreasonable to want to
/// look at, so `commit_detail`'s patch fetch is given more room than the
/// bound everything else shares. Still bounded, so a pathological blob
/// cannot take the editor down with it.
const COMMIT_PATCH_MAX_OUTPUT_BYTES: usize = 128 * 1024 * 1024;

/// The largest untracked file whose lines are counted for the changed-file
/// list.
///
/// Nothing here is a diff Git produced: the file is read to be counted, so a
/// build artefact nobody has ignored yet must not be read in full. Past this
/// size the file keeps its row and loses its numbers, which is the same
/// outcome as a binary one.
const MAX_UNTRACKED_STAT_BYTES: usize = 1024 * 1024;

/// How much is read across all untracked files while counting one status.
///
/// The per-file bound above says nothing about how many files there are, and a
/// working tree can hold thousands Git has not been told to ignore. Once this
/// is spent the rest keep their rows without numbers.
const MAX_UNTRACKED_STAT_BUDGET: usize = 16 * 1024 * 1024;

/// How much of a failure log is retained for its notification. This is large
/// enough for thousands of ordinary log lines while keeping a hostile hook
/// from consuming unbounded editor memory. Truncation is always explicit.
const MAX_STDERR_BYTES: usize = 1024 * 1024;
const STDERR_TRUNCATED: &[u8] = b"\n[Runyte truncated stderr after 1048576 bytes]";
const MAX_FAILURE_OUTPUT_BYTES: usize = 1024 * 1024;
const MAX_GIT_MARKER_BYTES: usize = 64 * 1024;
#[cfg(unix)]
const PIPE_READ_READY: i16 = libc::POLLIN;
#[cfg(not(unix))]
const PIPE_READ_READY: i16 = 0;
#[cfg(unix)]
const PIPE_WRITE_READY: i16 = libc::POLLOUT;
#[cfg(not(unix))]
const PIPE_WRITE_READY: i16 = 0;

struct PipeFinalizer {
    requested: Arc<AtomicBool>,
    #[cfg(unix)]
    wake_reader: Arc<std::os::unix::net::UnixStream>,
    #[cfg(unix)]
    wake_writer: std::sync::Mutex<Option<std::os::unix::net::UnixStream>>,
    #[cfg(test)]
    reader_gate: Option<Arc<TestPipeReaderGate>>,
    #[cfg(test)]
    poll_observer: Option<Arc<TestPipePollObserver>>,
}

#[derive(Clone)]
struct PipeFinishSignal {
    requested: Arc<AtomicBool>,
    #[cfg(unix)]
    wake_reader: Arc<std::os::unix::net::UnixStream>,
    #[cfg(test)]
    reader_gate: Option<Arc<TestPipeReaderGate>>,
    #[cfg(test)]
    poll_observer: Option<Arc<TestPipePollObserver>>,
}

#[cfg(test)]
#[derive(Debug, Default)]
struct TestPipeReaderGate {
    released: std::sync::Mutex<bool>,
    ready: std::sync::Condvar,
}

#[cfg(test)]
impl TestPipeReaderGate {
    fn wait(&self) {
        let mut released = self.released.lock().unwrap();
        while !*released {
            released = self.ready.wait(released).unwrap();
        }
    }

    fn release(&self) {
        *self.released.lock().unwrap() = true;
        self.ready.notify_all();
    }
}

#[cfg(test)]
#[derive(Debug, Default)]
struct TestPipePollObserver {
    entered: std::sync::Mutex<usize>,
    changed: std::sync::Condvar,
}

#[cfg(test)]
impl TestPipePollObserver {
    fn note(&self) {
        *self.entered.lock().unwrap() += 1;
        self.changed.notify_all();
    }

    fn wait_for(&self, expected: usize, timeout: std::time::Duration) -> bool {
        let deadline = std::time::Instant::now() + timeout;
        let mut entered = self.entered.lock().unwrap();
        while *entered < expected {
            let Some(remaining) = deadline.checked_duration_since(std::time::Instant::now()) else {
                return false;
            };
            let (next, result) = self.changed.wait_timeout(entered, remaining).unwrap();
            entered = next;
            if result.timed_out() && *entered < expected {
                return false;
            }
        }
        true
    }
}

trait NonblockingPipe {
    fn make_nonblocking(&self) -> io::Result<()>;

    #[cfg(unix)]
    fn raw_fd(&self) -> std::os::fd::RawFd;
}

#[cfg(unix)]
macro_rules! impl_nonblocking_pipe {
    ($($pipe:ty),+ $(,)?) => {
        $(
            impl NonblockingPipe for $pipe {
                fn make_nonblocking(&self) -> io::Result<()> {
                    set_nonblocking(self)
                }

                fn raw_fd(&self) -> std::os::fd::RawFd {
                    std::os::fd::AsRawFd::as_raw_fd(self)
                }
            }
        )+
    };
}

#[cfg(unix)]
impl_nonblocking_pipe!(
    std::process::ChildStdout,
    std::process::ChildStderr,
    std::process::ChildStdin,
);

#[cfg(test)]
impl NonblockingPipe for &[u8] {
    fn make_nonblocking(&self) -> io::Result<()> {
        Ok(())
    }

    #[cfg(unix)]
    fn raw_fd(&self) -> std::os::fd::RawFd {
        -1
    }
}

#[cfg(test)]
impl NonblockingPipe for &mut std::io::Cursor<&[u8]> {
    fn make_nonblocking(&self) -> io::Result<()> {
        Ok(())
    }

    #[cfg(unix)]
    fn raw_fd(&self) -> std::os::fd::RawFd {
        -1
    }
}

#[cfg(all(test, unix))]
impl NonblockingPipe for std::os::unix::net::UnixStream {
    fn make_nonblocking(&self) -> io::Result<()> {
        set_nonblocking(self)
    }

    fn raw_fd(&self) -> std::os::fd::RawFd {
        std::os::fd::AsRawFd::as_raw_fd(self)
    }
}

#[cfg(not(unix))]
macro_rules! impl_nonblocking_pipe {
    ($($pipe:ty),+ $(,)?) => {
        $(
            impl NonblockingPipe for $pipe {
                fn make_nonblocking(&self) -> io::Result<()> {
                    Ok(())
                }
            }
        )+
    };
}

#[cfg(not(unix))]
impl_nonblocking_pipe!(
    std::process::ChildStdout,
    std::process::ChildStderr,
    std::process::ChildStdin,
);

impl PipeFinalizer {
    fn new() -> io::Result<Self> {
        #[cfg(unix)]
        let (wake_reader, wake_writer) = std::os::unix::net::UnixStream::pair()?;
        #[cfg(unix)]
        {
            set_cloexec(&wake_reader)?;
            set_cloexec(&wake_writer)?;
        }
        Ok(Self {
            requested: Arc::new(AtomicBool::new(false)),
            #[cfg(unix)]
            wake_reader: Arc::new(wake_reader),
            #[cfg(unix)]
            wake_writer: std::sync::Mutex::new(Some(wake_writer)),
            #[cfg(test)]
            reader_gate: None,
            #[cfg(test)]
            poll_observer: None,
        })
    }

    #[cfg(test)]
    fn with_test_hooks(
        reader_gate: Option<Arc<TestPipeReaderGate>>,
        poll_observer: Option<Arc<TestPipePollObserver>>,
    ) -> io::Result<Self> {
        let mut finalizer = Self::new()?;
        finalizer.reader_gate = reader_gate;
        finalizer.poll_observer = poll_observer;
        Ok(finalizer)
    }

    fn signal(&self) -> PipeFinishSignal {
        PipeFinishSignal {
            requested: Arc::clone(&self.requested),
            #[cfg(unix)]
            wake_reader: Arc::clone(&self.wake_reader),
            #[cfg(test)]
            reader_gate: self.reader_gate.clone(),
            #[cfg(test)]
            poll_observer: self.poll_observer.clone(),
        }
    }

    fn release_reader_gate(&self) {
        #[cfg(test)]
        if let Some(gate) = &self.reader_gate {
            gate.release();
        }
    }

    #[cfg(unix)]
    fn finish(&self) {
        // `try_finish_child` has observed the leader without reaping it, ended
        // that still-anchored process group, and then collected its status.
        // The readers can now drain everything the owned group wrote and use
        // this signal only to escape a pipe retained by a descendant which
        // created a different session.
        self.request_finish();
        self.release_reader_gate();
    }

    fn request_finish(&self) {
        self.requested.store(true, Ordering::Release);
        #[cfg(unix)]
        self.wake_writer.lock().unwrap().take();
    }
}

impl Drop for PipeFinalizer {
    fn drop(&mut self) {
        self.request_finish();
        #[cfg(test)]
        if let Some(gate) = &self.reader_gate {
            gate.release();
        }
    }
}

impl PipeFinishSignal {
    fn wait_until_reader_may_start(&self) {
        #[cfg(test)]
        if let Some(gate) = &self.reader_gate {
            gate.wait();
        }
    }

    fn should_finish(&self) -> bool {
        if !self.requested.load(Ordering::Acquire) {
            return false;
        }
        true
    }

    fn note_poll(&self) {
        #[cfg(test)]
        if let Some(observer) = &self.poll_observer {
            observer.note();
        }
    }
}

#[cfg(unix)]
fn set_nonblocking(reader: &impl std::os::fd::AsRawFd) -> io::Result<()> {
    let descriptor = reader.as_raw_fd();
    let flags = unsafe { libc::fcntl(descriptor, libc::F_GETFL) };
    if flags == -1 {
        return Err(io::Error::last_os_error());
    }
    if unsafe { libc::fcntl(descriptor, libc::F_SETFL, flags | libc::O_NONBLOCK) } == -1 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(unix)]
fn set_cloexec(descriptor: &impl std::os::fd::AsRawFd) -> io::Result<()> {
    let descriptor = descriptor.as_raw_fd();
    let flags = unsafe { libc::fcntl(descriptor, libc::F_GETFD) };
    if flags == -1 {
        return Err(io::Error::last_os_error());
    }
    if unsafe { libc::fcntl(descriptor, libc::F_SETFD, flags | libc::FD_CLOEXEC) } == -1 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

fn wait_for_pipe(
    pipe: &impl NonblockingPipe,
    events: i16,
    signal: &PipeFinishSignal,
) -> io::Result<bool> {
    #[cfg(unix)]
    {
        use std::os::fd::AsRawFd;

        let mut descriptors = [
            libc::pollfd {
                fd: pipe.raw_fd(),
                events,
                revents: 0,
            },
            libc::pollfd {
                fd: signal.wake_reader.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            },
        ];
        signal.note_poll();
        loop {
            let ready = unsafe { libc::poll(descriptors.as_mut_ptr(), descriptors.len() as _, -1) };
            if ready >= 0 {
                // The finalizer is the causal boundary for the child and all
                // of its owned descendants. Darwin's poll adapter can report
                // stale pipe readiness at EOF, so a simultaneously ready wake
                // must move the reader into its final nonblocking drain first.
                if signal.should_finish() || descriptors[1].revents != 0 {
                    return Ok(!signal.should_finish());
                }
                if descriptors[0].revents != 0 {
                    return Ok(true);
                }
                continue;
            }
            let error = io::Error::last_os_error();
            if error.kind() != io::ErrorKind::Interrupted {
                return Err(error);
            }
        }
    }
    #[cfg(not(unix))]
    {
        let _ = (pipe, events);
        signal.note_poll();
        Ok(!signal.should_finish())
    }
}

fn read_bounded_stderr(
    mut reader: impl Read + NonblockingPipe,
    finish: &PipeFinishSignal,
) -> (Vec<u8>, bool) {
    let mut stderr = Vec::new();
    let mut truncated = false;
    let mut buffer = [0_u8; 8192];
    finish.wait_until_reader_may_start();
    if reader.make_nonblocking().is_err() {
        return (stderr, false);
    }
    let mut settled = false;
    let mut finalizing = false;
    loop {
        let read = match reader.read(&mut buffer) {
            Ok(0) => {
                settled = true;
                break;
            }
            Ok(read) => read,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) if error.kind() == io::ErrorKind::WouldBlock && finalizing => {
                settled = true;
                break;
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                match wait_for_pipe(&reader, PIPE_READ_READY, finish) {
                    Ok(true) => continue,
                    Ok(false) => {
                        finalizing = true;
                        continue;
                    }
                    Err(_) => break,
                }
            }
            Err(_) => break,
        };
        let retained = MAX_STDERR_BYTES.saturating_sub(stderr.len());
        stderr.extend_from_slice(&buffer[..read.min(retained)]);
        truncated |= read > retained;
    }
    if truncated {
        stderr.extend_from_slice(STDERR_TRUNCATED);
    }
    (stderr, settled)
}

/// Drains a child stream while retaining only enough bytes to classify it.
///
/// The reader must keep draining after the limit: closing a full pipe would
/// send SIGPIPE to Git or a hook and turn Runyte's presentation bound into a
/// change in command behavior. The parent observes `exceeded` and kills the
/// whole child process group instead.
fn read_bounded_output(
    mut reader: impl Read + NonblockingPipe,
    limit: usize,
    exceeded: &AtomicBool,
    finish: &PipeFinishSignal,
) -> (Vec<u8>, io::Result<()>, bool) {
    let mut output = Vec::new();
    let retained_limit = limit.saturating_add(1);
    let mut buffer = [0_u8; 8192];
    finish.wait_until_reader_may_start();
    if let Err(error) = reader.make_nonblocking() {
        return (output, Err(error), false);
    }
    let mut finalizing = false;
    loop {
        let read = match reader.read(&mut buffer) {
            Ok(0) => return (output, Ok(()), true),
            Ok(read) => read,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) if error.kind() == io::ErrorKind::WouldBlock && finalizing => {
                return (output, Ok(()), true);
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                match wait_for_pipe(&reader, PIPE_READ_READY, finish) {
                    Ok(true) => continue,
                    Ok(false) => {
                        finalizing = true;
                        continue;
                    }
                    Err(error) => return (output, Err(error), false),
                }
            }
            Err(error) => return (output, Err(error), false),
        };
        let retained = retained_limit.saturating_sub(output.len());
        output.extend_from_slice(&buffer[..read.min(retained)]);
        if output.len() > limit || read > retained {
            exceeded.store(true, Ordering::Release);
        }
    }
}

fn write_input(
    mut writer: impl Write + NonblockingPipe,
    input: &[u8],
    finish: &PipeFinishSignal,
) -> io::Result<()> {
    finish.wait_until_reader_may_start();
    writer.make_nonblocking()?;
    let mut written = 0;
    let mut finalizing = false;
    while written < input.len() {
        if finish.should_finish() {
            return Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "Git exited before consuming all command input",
            ));
        }
        match writer.write(&input[written..]) {
            Ok(0) => return Err(io::ErrorKind::WriteZero.into()),
            Ok(bytes) => written += bytes,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) if error.kind() == io::ErrorKind::WouldBlock && finalizing => {
                return Err(error);
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                if !wait_for_pipe(&writer, PIPE_WRITE_READY, finish)? {
                    finalizing = true;
                }
            }
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

fn failure_output(stdout: &[u8], stderr: &[u8]) -> String {
    let stdout = String::from_utf8_lossy(stdout);
    let stderr = String::from_utf8_lossy(stderr);
    failure_output_text(&stdout, &stderr)
}

fn failure_output_text(stdout: &str, stderr: &str) -> String {
    let stdout = stdout.trim();
    let stderr = stderr.trim();
    let streams = usize::from(!stdout.is_empty()) + usize::from(!stderr.is_empty());
    if streams == 0 {
        return String::new();
    }
    let header_bytes = usize::from(!stdout.is_empty()) * "stdout:\n".len()
        + usize::from(!stderr.is_empty()) * "stderr:\n".len()
        + usize::from(streams == 2);
    let budget = MAX_FAILURE_OUTPUT_BYTES.saturating_sub(header_bytes) / streams;
    let mut parts = Vec::with_capacity(streams);
    if !stdout.is_empty() {
        parts.push(format!(
            "stdout:\n{}",
            truncate_failure_stream(stdout, budget, "stdout")
        ));
    }
    if !stderr.is_empty() {
        parts.push(format!(
            "stderr:\n{}",
            truncate_failure_stream(stderr, budget, "stderr")
        ));
    }
    parts.join("\n")
}

fn truncate_failure_stream(value: &str, limit: usize, label: &str) -> String {
    if value.len() <= limit {
        return value.to_owned();
    }
    let marker = format!("\n[Runyte truncated {label} in failure output]");
    let mut end = limit.saturating_sub(marker.len()).min(value.len());
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    let mut output = value[..end].to_owned();
    output.push_str(&marker[..marker.len().min(limit.saturating_sub(output.len()))]);
    output
}

/// How long a network call may run before it is stopped.
///
/// A backstop rather than a service-level promise: a remote that answers slowly
/// should still be waited for, and one that has stopped answering at all should
/// not hold the editor until the operating system gives up on the socket. A
/// timed-out pull may already have updated remote-tracking refs during its
/// fetch, and a timed-out push may have reached the remote; the deadline bounds
/// the editor wait, not the operation's atomicity.
const NETWORK_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);

/// How often a running network call is checked against its deadline.
const NETWORK_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(50);

/// Local history and attribution reads are bounded in time as well as bytes.
const LOCAL_READ_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// The name looked for when no explicit program is configured.
const PROGRAM: &str = "git";

/// How much of a command is named back in a failure.
///
/// Arguments are diagnostic — which path, which object — but they are not
/// always small: a commit message is an argument, and a status line is not the
/// place to reprint one. What matters most in the message is what Git said,
/// which comes after this.
const MAX_DESCRIPTION_CHARS: usize = 80;

#[derive(Clone, Debug)]
pub struct GitCliProvider {
    program: PathBuf,
    max_output_bytes: usize,
    cancellation: Option<Arc<AtomicBool>>,
    local_read_timeout: std::time::Duration,
    #[cfg(test)]
    pipe_reader_gate: Option<Arc<TestPipeReaderGate>>,
    #[cfg(test)]
    pipe_poll_observer: Option<Arc<TestPipePollObserver>>,
}

impl GitCliProvider {
    pub fn new(program: impl Into<PathBuf>) -> Self {
        Self {
            program: program.into(),
            max_output_bytes: DEFAULT_MAX_OUTPUT_BYTES,
            cancellation: None,
            local_read_timeout: LOCAL_READ_TIMEOUT,
            #[cfg(test)]
            pipe_reader_gate: None,
            #[cfg(test)]
            pipe_poll_observer: None,
        }
    }

    /// Finds `git` on the supplied search path, without running it.
    ///
    /// The path is a parameter rather than an environment read so tests and
    /// headless hosts can answer the question without touching the process
    /// environment, and so the empty-entry rule that keeps a repository from
    /// supplying its own `git` applies here too.
    pub fn discover(search_path: Option<&OsStr>) -> Option<Self> {
        crate::service_health::resolve_configured_executable(Path::new(PROGRAM), search_path)
            .map(Self::new)
    }

    /// Finds `git` on the current process's `PATH`.
    pub fn from_environment() -> Option<Self> {
        Self::discover(std::env::var_os("PATH").as_deref())
    }

    /// Raises or lowers the output bound for one provider.
    ///
    /// Used where a caller legitimately expects more than the default, such as
    /// a whole-worktree patch.
    #[must_use]
    pub fn with_max_output_bytes(mut self, bytes: usize) -> Self {
        self.max_output_bytes = bytes;
        self
    }

    /// The bound applied to one commit's patch text.
    ///
    /// A commit's diff is allowed past the shared default, since a large but
    /// ordinary commit can exceed it without being unreasonable to want to
    /// look at. That relief only widens the *default*: a caller who has
    /// lowered the provider's bound below the default has asked for a
    /// tighter resource cap than usual, and this must not raise it back up
    /// on their behalf. A caller who raised it instead is still honored, via
    /// the `max`.
    fn patch_output_bytes(&self) -> usize {
        if self.max_output_bytes < DEFAULT_MAX_OUTPUT_BYTES {
            self.max_output_bytes
        } else {
            self.max_output_bytes.max(COMMIT_PATCH_MAX_OUTPUT_BYTES)
        }
    }

    pub fn program(&self) -> &Path {
        &self.program
    }

    /// Returns a worker-local provider whose owned subprocesses observe the
    /// supplied cancellation flag.
    #[must_use]
    pub(crate) fn with_cancellation(mut self, cancellation: Arc<AtomicBool>) -> Self {
        self.cancellation = Some(cancellation);
        self
    }

    /// The same provider with the cancellation flag dropped, for the cleanup
    /// that has to run *because* an operation was stopped.
    ///
    /// A flag that stopped a command stops every command made after it on the
    /// same provider, cleanup included: the probe that asks whether anything
    /// needs undoing would be killed before it answered, and the undo itself
    /// before it finished. That is the one case where inheriting the flag
    /// produces exactly the abandoned state cancelling was meant to avoid.
    /// Uses of this must stay bounded and few — undoing a half-applied
    /// operation, never continuing one.
    #[must_use]
    fn uncancellable(&self) -> Self {
        Self {
            cancellation: None,
            ..self.clone()
        }
    }

    #[cfg(test)]
    pub(crate) fn with_local_read_timeout(mut self, timeout: std::time::Duration) -> Self {
        self.local_read_timeout = timeout;
        self
    }

    #[cfg(test)]
    fn with_pipe_reader_gate(mut self, gate: Arc<TestPipeReaderGate>) -> Self {
        self.pipe_reader_gate = Some(gate);
        self
    }

    #[cfg(test)]
    fn with_pipe_poll_observer(mut self, observer: Arc<TestPipePollObserver>) -> Self {
        self.pipe_poll_observer = Some(observer);
        self
    }

    fn pipe_finalizer(&self, directory: &Path) -> Result<PipeFinalizer> {
        #[cfg(test)]
        let finalizer = PipeFinalizer::with_test_hooks(
            self.pipe_reader_gate.clone(),
            self.pipe_poll_observer.clone(),
        );
        #[cfg(not(test))]
        let finalizer = PipeFinalizer::new();
        finalizer.map_err(|error| GitError::Io {
            action: "prepare Git output readers in",
            path: directory.to_path_buf(),
            detail: error.to_string(),
        })
    }

    /// Runs Git in `directory` and returns its standard output.
    pub fn run<S: AsRef<OsStr>>(&self, directory: &Path, arguments: &[S]) -> Result<Vec<u8>> {
        self.run_bounded(directory, arguments, self.max_output_bytes)
    }

    /// Runs Git and returns its output with trailing whitespace removed.
    pub fn run_text<S: AsRef<OsStr>>(&self, directory: &Path, arguments: &[S]) -> Result<String> {
        let output = self.run(directory, arguments)?;
        Ok(self.utf8(arguments, output)?.trim_end().to_owned())
    }

    /// Runs Git and returns its output exactly, keeping trailing newlines.
    pub fn run_raw_text<S: AsRef<OsStr>>(
        &self,
        directory: &Path,
        arguments: &[S],
    ) -> Result<String> {
        let output = self.run(directory, arguments)?;
        self.utf8(arguments, output)
    }

    /// Runs Git with an explicit output bound for this call alone.
    pub fn run_bounded<S: AsRef<OsStr>>(
        &self,
        directory: &Path,
        arguments: &[S],
        limit: usize,
    ) -> Result<Vec<u8>> {
        let described = self.describe(arguments);
        let pipe_finalizer = self.pipe_finalizer(directory)?;
        let (mut child, child_exit) = self.spawn(directory, arguments, false, false)?;

        // Both pipes are drained away from the worker so it can observe
        // cancellation even when Git or one of its hooks is still running.
        let stderr_pipe = child.stderr.take().expect("stderr was piped");
        let stderr_finish = pipe_finalizer.signal();
        let stderr_reader =
            std::thread::spawn(move || read_bounded_stderr(stderr_pipe, &stderr_finish));

        let stdout_pipe = child.stdout.take().expect("stdout was piped");
        let output_exceeded = Arc::new(AtomicBool::new(false));
        let reader_exceeded = Arc::clone(&output_exceeded);
        let stdout_finish = pipe_finalizer.signal();
        let stdout_reader = std::thread::spawn(move || {
            read_bounded_output(stdout_pipe, limit, &reader_exceeded, &stdout_finish)
        });

        let status = loop {
            match try_finish_child(&mut child, &child_exit) {
                Ok(Some(status)) => break status,
                Ok(None) => {}
                Err(error) => {
                    stop_child_tree(&mut child);
                    return Err(GitError::Io {
                        action: "wait for Git in",
                        path: directory.to_path_buf(),
                        detail: error.to_string(),
                    });
                }
            }
            if self.is_cancelled() {
                stop_child_tree(&mut child);
                return Err(GitError::Cancelled { command: described });
            }
            if output_exceeded.load(Ordering::Acquire) {
                stop_child_tree(&mut child);
                return Err(GitError::TooLarge {
                    command: described,
                    limit,
                });
            }
            std::thread::sleep(NETWORK_POLL_INTERVAL);
        };
        #[cfg(not(unix))]
        pipe_finalizer.release_reader_gate();
        #[cfg(unix)]
        pipe_finalizer.finish();
        #[cfg(not(unix))]
        if !finish_readers_or_stop(&mut child, || {
            stdout_reader.is_finished() && stderr_reader.is_finished()
        }) {
            return Err(unclosed_output_error(directory));
        }
        let (stdout, read, stdout_settled) = stdout_reader.join().map_err(|_| GitError::Io {
            action: "read the output of Git in",
            path: directory.to_path_buf(),
            detail: "Git output reader stopped unexpectedly".to_owned(),
        })?;
        let (stderr, stderr_settled) = stderr_reader.join().unwrap_or_default();

        read.map_err(|error| GitError::Io {
            action: "read the output of Git in",
            path: directory.to_path_buf(),
            detail: error.to_string(),
        })?;
        if !stdout_settled || !stderr_settled {
            return Err(unclosed_output_error(directory));
        }
        if stdout.len() > limit || output_exceeded.load(Ordering::Acquire) {
            stop_child_tree(&mut child);
            return Err(GitError::TooLarge {
                command: described,
                limit,
            });
        }
        if !status.success() {
            return Err(GitError::Failed {
                command: described,
                code: status.code(),
                stderr: failure_output(&stdout, &stderr),
            });
        }
        Ok(stdout)
    }

    fn run_read_bounded<S: AsRef<OsStr>>(
        &self,
        directory: &Path,
        arguments: &[S],
        limit: usize,
    ) -> Result<Vec<u8>> {
        self.run_bounded_until(directory, arguments, limit, self.local_read_timeout)
    }

    fn run_bounded_until<S: AsRef<OsStr>>(
        &self,
        directory: &Path,
        arguments: &[S],
        limit: usize,
        timeout: std::time::Duration,
    ) -> Result<Vec<u8>> {
        let described = self.describe(arguments);
        let pipe_finalizer = self.pipe_finalizer(directory)?;
        let (mut child, child_exit) = self.spawn(directory, arguments, false, false)?;
        let stderr_pipe = child.stderr.take().expect("stderr was piped");
        let stderr_finish = pipe_finalizer.signal();
        let stderr_reader =
            std::thread::spawn(move || read_bounded_stderr(stderr_pipe, &stderr_finish));
        let stdout_pipe = child.stdout.take().expect("stdout was piped");
        let output_exceeded = Arc::new(AtomicBool::new(false));
        let reader_exceeded = Arc::clone(&output_exceeded);
        let stdout_finish = pipe_finalizer.signal();
        let stdout_reader = std::thread::spawn(move || {
            read_bounded_output(stdout_pipe, limit, &reader_exceeded, &stdout_finish)
        });
        let started = std::time::Instant::now();
        let status = loop {
            match try_finish_child(&mut child, &child_exit) {
                Ok(Some(status)) => break status,
                Ok(None) => {}
                Err(error) => {
                    stop_child_tree(&mut child);
                    return Err(GitError::Io {
                        action: "wait for Git in",
                        path: directory.to_path_buf(),
                        detail: error.to_string(),
                    });
                }
            }
            if self.is_cancelled() {
                stop_child_tree(&mut child);
                return Err(GitError::Cancelled { command: described });
            }
            if output_exceeded.load(Ordering::Acquire) {
                stop_child_tree(&mut child);
                return Err(GitError::TooLarge {
                    command: described,
                    limit,
                });
            }
            if started.elapsed() >= timeout {
                stop_child_tree(&mut child);
                return Err(GitError::TimedOut {
                    command: described,
                    seconds: timeout.as_secs(),
                });
            }
            std::thread::sleep(NETWORK_POLL_INTERVAL);
        };
        #[cfg(not(unix))]
        pipe_finalizer.release_reader_gate();
        #[cfg(unix)]
        pipe_finalizer.finish();
        #[cfg(not(unix))]
        if !finish_readers_or_stop(&mut child, || {
            stdout_reader.is_finished() && stderr_reader.is_finished()
        }) {
            return Err(unclosed_output_error(directory));
        }
        let (stdout, read, stdout_settled) = stdout_reader.join().map_err(|_| GitError::Io {
            action: "read the output of Git in",
            path: directory.to_path_buf(),
            detail: "Git output reader stopped unexpectedly".to_owned(),
        })?;
        let (stderr, stderr_settled) = stderr_reader.join().unwrap_or_default();
        read.map_err(|error| GitError::Io {
            action: "read the output of Git in",
            path: directory.to_path_buf(),
            detail: error.to_string(),
        })?;
        if !stdout_settled || !stderr_settled {
            return Err(unclosed_output_error(directory));
        }
        if stdout.len() > limit || output_exceeded.load(Ordering::Acquire) {
            stop_child_tree(&mut child);
            return Err(GitError::TooLarge {
                command: described,
                limit,
            });
        }
        if !status.success() {
            return Err(GitError::Failed {
                command: described,
                code: status.code(),
                stderr: failure_output(&stdout, &stderr),
            });
        }
        Ok(stdout)
    }

    fn run_with_input_bounded<S: AsRef<OsStr>>(
        &self,
        directory: &Path,
        arguments: &[S],
        input: &[u8],
        limit: usize,
    ) -> Result<Vec<u8>> {
        let described = self.describe(arguments);
        let pipe_finalizer = self.pipe_finalizer(directory)?;
        let (mut child, child_exit) = self.spawn(directory, arguments, false, true)?;
        let stdin = child.stdin.take().expect("stdin was piped");
        let input = input.to_vec();
        let stdin_finish = pipe_finalizer.signal();
        let stdin_writer = std::thread::spawn(move || write_input(stdin, &input, &stdin_finish));
        let stderr_pipe = child.stderr.take().expect("stderr was piped");
        let stderr_finish = pipe_finalizer.signal();
        let stderr_reader =
            std::thread::spawn(move || read_bounded_stderr(stderr_pipe, &stderr_finish));
        let stdout_pipe = child.stdout.take().expect("stdout was piped");
        let output_exceeded = Arc::new(AtomicBool::new(false));
        let reader_exceeded = Arc::clone(&output_exceeded);
        let stdout_finish = pipe_finalizer.signal();
        let stdout_reader = std::thread::spawn(move || {
            read_bounded_output(stdout_pipe, limit, &reader_exceeded, &stdout_finish)
        });
        let started = std::time::Instant::now();
        let status = loop {
            match try_finish_child(&mut child, &child_exit) {
                Ok(Some(status)) => break status,
                Ok(None) => {}
                Err(error) => {
                    stop_child_tree(&mut child);
                    return Err(GitError::Io {
                        action: "wait for Git in",
                        path: directory.to_path_buf(),
                        detail: error.to_string(),
                    });
                }
            }
            if self.is_cancelled() {
                stop_child_tree(&mut child);
                return Err(GitError::Cancelled { command: described });
            }
            if output_exceeded.load(Ordering::Acquire) {
                stop_child_tree(&mut child);
                return Err(GitError::TooLarge {
                    command: described,
                    limit,
                });
            }
            if started.elapsed() >= self.local_read_timeout {
                stop_child_tree(&mut child);
                return Err(GitError::TimedOut {
                    command: described,
                    seconds: self.local_read_timeout.as_secs(),
                });
            }
            std::thread::sleep(NETWORK_POLL_INTERVAL);
        };
        #[cfg(not(unix))]
        pipe_finalizer.release_reader_gate();
        #[cfg(unix)]
        pipe_finalizer.finish();
        #[cfg(not(unix))]
        if !finish_readers_or_stop(&mut child, || {
            stdin_writer.is_finished() && stdout_reader.is_finished() && stderr_reader.is_finished()
        }) {
            return Err(unclosed_output_error(directory));
        }
        let written = stdin_writer.join().map_err(|_| GitError::Io {
            action: "write the input of Git in",
            path: directory.to_path_buf(),
            detail: "Git input writer stopped unexpectedly".to_owned(),
        })?;
        written.map_err(|error| GitError::Io {
            action: "write the input of Git in",
            path: directory.to_path_buf(),
            detail: error.to_string(),
        })?;
        let (stdout, read, stdout_settled) = stdout_reader.join().map_err(|_| GitError::Io {
            action: "read the output of Git in",
            path: directory.to_path_buf(),
            detail: "Git output reader stopped unexpectedly".to_owned(),
        })?;
        let (stderr, stderr_settled) = stderr_reader.join().unwrap_or_default();
        read.map_err(|error| GitError::Io {
            action: "read the output of Git in",
            path: directory.to_path_buf(),
            detail: error.to_string(),
        })?;
        if !stdout_settled || !stderr_settled {
            return Err(unclosed_output_error(directory));
        }
        if stdout.len() > limit || output_exceeded.load(Ordering::Acquire) {
            stop_child_tree(&mut child);
            return Err(GitError::TooLarge {
                command: described,
                limit,
            });
        }
        if !status.success() {
            return Err(GitError::Failed {
                command: described,
                code: status.code(),
                stderr: failure_output(&stdout, &stderr),
            });
        }
        Ok(stdout)
    }

    /// Starts Git in `directory` with the environment every call is given.
    ///
    /// `network` hardens the extra ways a call that leaves the machine can stop
    /// and wait for a person. Runyte owns the terminal while this runs, so
    /// anything that tries to read from it would both corrupt the display and
    /// hang the frame, and failing is the better of the two outcomes.
    fn spawn<S: AsRef<OsStr>>(
        &self,
        directory: &Path,
        arguments: &[S],
        network: bool,
        pipe_stdin: bool,
    ) -> Result<(std::process::Child, ChildExitObserver)> {
        if !directory.is_dir() {
            return Err(GitError::Io {
                action: "start Git in",
                path: directory.to_path_buf(),
                detail: "the working directory is not a directory".to_owned(),
            });
        }
        let mut command = self.command(directory, arguments, network, pipe_stdin);
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            // Hooks and filters can outlive Git just like network helpers.
            // Every service-owned command therefore gets a process group that
            // cancellation can terminate as one unit.
            // SAFETY: `setsid` is async-signal-safe and only changes the child
            // process's session before exec.
            unsafe {
                command.pre_exec(|| {
                    if libc::setsid() == -1 {
                        return Err(std::io::Error::last_os_error());
                    }
                    Ok(())
                });
            }
        }
        let child = command.spawn().map_err(|error| GitError::Unavailable {
            detail: format!("cannot start `{}`: {error}", self.program.display()),
        })?;
        #[cfg(target_os = "macos")]
        let (child, observer) = {
            let mut child = child;
            let observer = ChildExitObserver::new(&child).map_err(|error| {
                stop_child_tree(&mut child);
                GitError::Io {
                    action: "observe Git process in",
                    path: directory.to_path_buf(),
                    detail: error.to_string(),
                }
            })?;
            (child, observer)
        };
        #[cfg(not(target_os = "macos"))]
        let observer = ChildExitObserver;
        Ok((child, observer))
    }

    fn command<S: AsRef<OsStr>>(
        &self,
        directory: &Path,
        arguments: &[S],
        network: bool,
        pipe_stdin: bool,
    ) -> Command {
        let mut command = Command::new(&self.program);
        // `--no-optional-locks` keeps a read from taking the index lock, so
        // asking what changed can never collide with a Git command the person
        // is running in another window.
        command
            .arg("--no-optional-locks")
            .args(arguments)
            .current_dir(directory)
            .stdin(if pipe_stdin {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            // Runyte may itself have been started from a hook or an alias.
            // These variables would silently retarget every call below at
            // some other repository or index.
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_INDEX_FILE")
            .env_remove("GIT_OBJECT_DIRECTORY")
            .env_remove("GIT_ALTERNATE_OBJECT_DIRECTORIES")
            .env_remove("GIT_COMMON_DIR")
            .env_remove("GIT_NAMESPACE")
            .env_remove("GIT_REPLACE_REF_BASE")
            .env_remove("GIT_SHALLOW_FILE")
            .env_remove("GIT_GRAFT_FILE")
            // Config injected by an outer Git command must not become
            // repository authority for this independent operation.
            .env_remove("GIT_CONFIG_PARAMETERS")
            .env_remove("GIT_CONFIG_COUNT")
            // Use the helpers installed alongside the resolved Git binary,
            // not an inherited command-specific replacement directory.
            .env_remove("GIT_EXEC_PATH")
            .env_remove("GIT_TEMPLATE_DIR")
            // Nothing here can answer a prompt, and a blocked credential
            // helper would hang the editor rather than fail it.
            .env("GIT_TERMINAL_PROMPT", "0");
        if network {
            // `GIT_TERMINAL_PROMPT` governs Git's own prompts, not SSH's: a key
            // with a passphrase and no agent would still stop and ask on the
            // terminal Runyte is drawing to. A configured command is left
            // alone, because someone who set one has already decided how their
            // authentication works.
            if std::env::var_os("GIT_SSH_COMMAND").is_none() {
                command.env("GIT_SSH_COMMAND", "ssh -o BatchMode=yes");
            }
            // An askpass helper opening a window is fine; the default of
            // falling back to the terminal is not.
            command.env("GIT_ASKPASS", "");
        }
        remove_inherited_config_entries(&mut command, std::env::vars_os().map(|(name, _)| name));
        command
    }

    /// Runs a Git command that reaches the network, and gives up on it.
    ///
    /// Both pipes are read on their own threads because this one cannot sit in
    /// a blocking read: the point of the deadline is that a remote which never
    /// answers stops being Runyte's problem, and a reader blocked on a pipe
    /// that will never close would wait exactly as long as the remote does.
    ///
    /// The returned text is standard output when there is any and standard
    /// error otherwise, because Git splits its report between them by command:
    /// a merge summary arrives on the first, a push report on the second.
    fn run_network<S: AsRef<OsStr>>(&self, directory: &Path, arguments: &[S]) -> Result<String> {
        self.run_network_with_timeout(directory, arguments, NETWORK_TIMEOUT)
    }

    fn run_network_with_timeout<S: AsRef<OsStr>>(
        &self,
        directory: &Path,
        arguments: &[S],
        timeout: std::time::Duration,
    ) -> Result<String> {
        let described = self.describe(arguments);
        let pipe_finalizer = self.pipe_finalizer(directory)?;
        let (mut child, child_exit) = self.spawn(directory, arguments, true, false)?;
        let limit = self.max_output_bytes;

        let stdout_pipe = child.stdout.take().expect("stdout was piped");
        let output_exceeded = Arc::new(AtomicBool::new(false));
        let reader_exceeded = Arc::clone(&output_exceeded);
        let stdout_finish = pipe_finalizer.signal();
        let stdout_reader = std::thread::spawn(move || {
            read_bounded_output(stdout_pipe, limit, &reader_exceeded, &stdout_finish)
        });
        let stderr_pipe = child.stderr.take().expect("stderr was piped");
        let stderr_finish = pipe_finalizer.signal();
        let stderr_reader =
            std::thread::spawn(move || read_bounded_stderr(stderr_pipe, &stderr_finish));

        let deadline = std::time::Instant::now() + timeout;
        let status = loop {
            match try_finish_child(&mut child, &child_exit) {
                Ok(Some(status)) => break status,
                Ok(None) => {}
                Err(error) => {
                    stop_child_tree(&mut child);
                    return Err(GitError::Io {
                        action: "wait for Git in",
                        path: directory.to_path_buf(),
                        detail: error.to_string(),
                    });
                }
            }
            if std::time::Instant::now() >= deadline {
                stop_child_tree(&mut child);
                // Do not join the readers on the timeout path. Unix has just
                // killed the process group, but on other platforms a helper
                // descendant may still own a copied pipe handle. Dropping the
                // join handles detaches those readers and, crucially, keeps a
                // helper from extending the editor's advertised deadline.
                return Err(GitError::TimedOut {
                    command: described,
                    seconds: timeout.as_secs(),
                });
            }
            if self.is_cancelled() {
                stop_child_tree(&mut child);
                return Err(GitError::Cancelled { command: described });
            }
            if output_exceeded.load(Ordering::Acquire) {
                stop_child_tree(&mut child);
                return Err(GitError::TooLarge {
                    command: described,
                    limit,
                });
            }
            std::thread::sleep(NETWORK_POLL_INTERVAL);
        };

        #[cfg(not(unix))]
        pipe_finalizer.release_reader_gate();
        #[cfg(unix)]
        pipe_finalizer.finish();
        #[cfg(not(unix))]
        if !finish_readers_or_stop(&mut child, || {
            stdout_reader.is_finished() && stderr_reader.is_finished()
        }) {
            return Err(unclosed_output_error(directory));
        }
        let (stdout, read, stdout_settled) = stdout_reader.join().map_err(|_| GitError::Io {
            action: "read the output of Git in",
            path: directory.to_path_buf(),
            detail: "Git output reader stopped unexpectedly".to_owned(),
        })?;
        let (stderr, stderr_settled) = stderr_reader.join().unwrap_or_default();
        read.map_err(|error| GitError::Io {
            action: "read the output of Git in",
            path: directory.to_path_buf(),
            detail: error.to_string(),
        })?;
        if !stdout_settled || !stderr_settled {
            return Err(unclosed_output_error(directory));
        }
        if stdout.len() > limit || output_exceeded.load(Ordering::Acquire) {
            stop_child_tree(&mut child);
            return Err(GitError::TooLarge {
                command: described,
                limit,
            });
        }
        if !status.success() {
            let stderr = without_noise(&String::from_utf8_lossy(&stderr));
            return Err(GitError::Failed {
                command: described,
                code: status.code(),
                stderr: failure_output_text(&String::from_utf8_lossy(&stdout), &stderr),
            });
        }
        let stdout = String::from_utf8_lossy(&stdout).trim().to_owned();
        Ok(if stdout.is_empty() {
            settled(&String::from_utf8_lossy(&stderr))
        } else {
            stdout
        })
    }

    fn is_cancelled(&self) -> bool {
        self.cancellation
            .as_ref()
            .is_some_and(|flag| flag.load(Ordering::Acquire))
    }

    fn utf8<S: AsRef<OsStr>>(&self, arguments: &[S], output: Vec<u8>) -> Result<String> {
        String::from_utf8(output).map_err(|_| GitError::Malformed {
            command: self.describe(arguments),
            detail: "Git produced output that is not UTF-8".to_owned(),
        })
    }

    /// Where to publish a branch that tracks nothing yet.
    ///
    /// `origin` when it exists, because that is what a clone calls the place it
    /// came from; otherwise the only remote, when there is exactly one. Any
    /// other arrangement is a choice Runyte should not make silently, so it
    /// says what it found instead.
    fn default_remote(&self, repository: &Repository) -> Result<String> {
        let remotes = self.run_text(repository.workdir(), &["remote"])?;
        let remotes = remotes
            .lines()
            .map(str::trim)
            .filter(|remote| !remote.is_empty())
            .collect::<Vec<_>>();
        if remotes.contains(&"origin") {
            return Ok("origin".to_owned());
        }
        match remotes.as_slice() {
            [only] => Ok((*only).to_owned()),
            [] => Err(GitError::Failed {
                command: "git push".to_owned(),
                code: None,
                stderr: "this repository has no remote to push to".to_owned(),
            }),
            many => Err(GitError::Failed {
                command: "git push".to_owned(),
                code: None,
                stderr: format!(
                    "this branch tracks nothing and there is no `origin` to choose: set an \
                     upstream, or push to one of {}",
                    many.join(", ")
                ),
            }),
        }
    }

    /// The current branch, once it is established that there is one and that
    /// it tracks something.
    ///
    /// Pull and rebase share these three preconditions and differ only in the
    /// verb the refusal reads with, so the checks live here rather than being
    /// written out twice with a chance of drifting apart.
    fn upstream_branch(
        &self,
        repository: &Repository,
        command: &str,
        into: &str,
        from: &str,
    ) -> Result<String> {
        let status = self.status(repository)?;
        let branch = match status.head {
            Head::Branch(branch) => branch,
            Head::Unborn(_) => {
                return Err(GitError::Failed {
                    command: command.to_owned(),
                    code: None,
                    stderr: "this branch has no commits yet".to_owned(),
                });
            }
            Head::Detached(_) => {
                return Err(GitError::Failed {
                    command: command.to_owned(),
                    code: None,
                    stderr: format!("HEAD is detached, so there is no branch to {into}"),
                });
            }
        };
        if status.upstream.is_none() {
            return Err(GitError::Failed {
                command: command.to_owned(),
                code: None,
                stderr: format!("this branch tracks nothing to {from}"),
            });
        }
        Ok(branch)
    }

    /// How the current branch has drifted from its upstream, as an error, when
    /// it has moved both ways and so has no fast-forward in either direction.
    ///
    /// Answers from the refs Git already holds, so it reports the drift as of
    /// the last fetch rather than reaching the network again.
    fn divergence(&self, repository: &Repository) -> Option<GitError> {
        let branches = self.branches(repository).ok()?;
        let branch = branches.into_iter().find(|branch| branch.current)?;
        let upstream = branch.upstream?;
        let divergence = upstream.divergence?;
        (divergence.ahead > 0 && divergence.behind > 0).then_some(GitError::Diverged {
            branch: branch.name,
            upstream: upstream.name,
            ahead: divergence.ahead,
            behind: divergence.behind,
        })
    }

    /// Whether a rebase is stopped partway through this working tree.
    ///
    /// `--git-path` rather than a path built onto the common directory: rebase
    /// state belongs to one worktree rather than to the repository, and Git is
    /// the one that knows where a linked worktree keeps it.
    fn rebase_in_progress(&self, repository: &Repository) -> bool {
        ["rebase-merge", "rebase-apply"].iter().any(|state| {
            self.run_text(repository.workdir(), &["rev-parse", "--git-path", state])
                .is_ok_and(|path| {
                    let path = Path::new(path.trim());
                    if path.is_absolute() {
                        path.exists()
                    } else {
                        repository.workdir().join(path).exists()
                    }
                })
        })
    }

    /// The lines one side of the index adds and removes, per path.
    ///
    /// `--numstat` rather than `--stat`: the numbers are exact where `--stat`
    /// scales them into a bar sized for a terminal Git guessed at, and the
    /// external diff drivers are refused here for the same reason they are in
    /// [`GitProvider::diff`] — a checkout must not decide what program runs.
    fn numstat(
        &self,
        repository: &Repository,
        scope: DiffScope,
    ) -> Result<Vec<(PathBuf, LineStats)>> {
        let mut arguments = vec![
            OsString::from("diff"),
            OsString::from("--no-ext-diff"),
            OsString::from("--no-textconv"),
            OsString::from("--numstat"),
            OsString::from("-z"),
        ];
        if scope == DiffScope::Staged {
            arguments.push(OsString::from("--cached"));
        }
        let output = self.run(repository.workdir(), &arguments)?;
        parse_numstat(&output).map_err(|detail| GitError::Malformed {
            command: self.describe(&arguments),
            detail,
        })
    }

    /// The repository-relative form of a path, refusing one that is elsewhere.
    fn relative<'a>(&self, repository: &Repository, path: &'a Path) -> Result<&'a Path> {
        let relative = repository
            .relative(path)
            .ok_or_else(|| GitError::NotARepository {
                path: path.to_path_buf(),
            })?;
        if relative.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        }) {
            return Err(GitError::NotARepository {
                path: path.to_path_buf(),
            });
        }
        Ok(relative)
    }

    fn head_content(&self, repository: &Repository, path: &Path) -> Result<BaseContent> {
        if matches!(self.status(repository)?.head, Head::Unborn(_)) {
            return Ok(BaseContent::Absent);
        }
        let relative = self.relative(repository, path)?;
        let entries = self.run(
            repository.workdir(),
            &[
                OsStr::new("--literal-pathspecs"),
                OsStr::new("ls-tree"),
                OsStr::new("-z"),
                OsStr::new("HEAD"),
                OsStr::new("--"),
                relative.as_os_str(),
            ],
        )?;
        let Some(object) = tree_object(&entries) else {
            return Ok(BaseContent::Absent);
        };
        self.object_content(repository, &object)
    }

    fn object_content(&self, repository: &Repository, object: &str) -> Result<BaseContent> {
        let content = self.run(
            repository.workdir(),
            &[
                OsStr::new("cat-file"),
                OsStr::new("blob"),
                OsStr::new(object),
            ],
        )?;
        if crate::external_open::is_binary(&content, true) {
            return Ok(BaseContent::Binary);
        }
        Ok(String::from_utf8(content)
            .map(BaseContent::Text)
            .unwrap_or(BaseContent::Binary))
    }

    fn working_content(&self, repository: &Repository, path: &Path) -> Result<BaseContent> {
        self.relative(repository, path)?;
        let metadata = match std::fs::symlink_metadata(path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(BaseContent::Absent);
            }
            Err(error) => {
                return Err(GitError::Io {
                    action: "read",
                    path: path.to_path_buf(),
                    detail: error.to_string(),
                });
            }
        };
        if !metadata.file_type().is_file() {
            return Ok(BaseContent::Binary);
        }
        crate::path_safety::ensure_within_root(repository.workdir(), path).map_err(|_| {
            GitError::NotARepository {
                path: path.to_path_buf(),
            }
        })?;
        let content = read_file_for_comparison(path, self.max_output_bytes)?;
        if crate::external_open::is_binary(&content, true) {
            return Ok(BaseContent::Binary);
        }
        Ok(String::from_utf8(content)
            .map(BaseContent::Text)
            .unwrap_or(BaseContent::Binary))
    }

    fn describe<S: AsRef<OsStr>>(&self, arguments: &[S]) -> String {
        let mut described = OsString::from(PROGRAM);
        for argument in arguments {
            described.push(" ");
            described.push(argument);
        }
        let described = described.to_string_lossy();
        match described.char_indices().nth(MAX_DESCRIPTION_CHARS) {
            Some((boundary, _)) => format!("{}…", &described[..boundary]),
            None => described.into_owned(),
        }
    }
}

fn remove_inherited_config_entries(
    command: &mut Command,
    names: impl IntoIterator<Item = OsString>,
) {
    for name in names {
        let name_text = name.to_string_lossy();
        if name_text.starts_with("GIT_CONFIG_KEY_") || name_text.starts_with("GIT_CONFIG_VALUE_") {
            command.env_remove(name);
        }
    }
}

/// Reads a regular file that Git will not read for us, refusing one that is
/// too large.
///
/// `None` covers every reason there are no bytes to count — the file is gone,
/// it is not a regular file, it cannot be opened, or it is past the bound —
/// because the row is shown without numbers in all of them and a status list
/// is not where a reader wants to be told why an untracked path could not be
/// measured. Symlinks are deliberately not followed: their target may be
/// outside the repository, and Git would stage the link rather than that
/// target's contents.
fn read_bounded_file(root: &Path, path: &Path, limit: usize) -> Option<Vec<u8>> {
    let metadata = std::fs::symlink_metadata(path).ok()?;
    if !metadata.file_type().is_file() || metadata.len() > limit as u64 {
        return None;
    }
    crate::path_safety::ensure_within_root(root, path).ok()?;
    let file = std::fs::File::open(path).ok()?;
    let mut content = Vec::new();
    let mut bounded = file.take(limit as u64);
    bounded.read_to_end(&mut content).ok()?;
    // Recheck the open file so growth during the read cannot turn a truncated
    // prefix into a plausible line count.
    let final_length = bounded.get_ref().metadata().ok()?.len();
    (final_length == content.len() as u64).then_some(content)
}

#[cfg(target_os = "macos")]
struct ChildExitObserver {
    pid: libc::pid_t,
    already_zombie: bool,
}

#[cfg(not(target_os = "macos"))]
struct ChildExitObserver;

#[cfg(target_os = "macos")]
impl ChildExitObserver {
    fn new(child: &std::process::Child) -> io::Result<Self> {
        let pid = i32::try_from(child.id()).map_err(|_| {
            io::Error::new(io::ErrorKind::InvalidInput, "child PID does not fit pid_t")
        })?;
        // A direct, unreaped Child remains in either XNU's live or zombie
        // process table. Query both tables so the observer is level-triggered
        // and has no spawn-to-registration gap. INEXIT precedes SZOMB and may
        // not authorize group cleanup: signalling the group then would also
        // signal the still-exiting Git leader and could replace its successful
        // status with SIGKILL.
        let already_zombie = darwin_process_is_zombie(pid)?;
        Ok(Self {
            pid,
            already_zombie,
        })
    }

    fn exited(&self) -> io::Result<bool> {
        if self.already_zombie {
            return Ok(true);
        }
        darwin_process_is_zombie(self.pid)
    }
}

#[cfg(target_os = "macos")]
const DARWIN_PROC_FLAG_INEXIT: u32 = 4;

#[cfg(target_os = "macos")]
const DARWIN_INCLUDE_ZOMBIES: u64 = 1;

#[cfg(target_os = "macos")]
fn darwin_process_is_zombie(pid: libc::pid_t) -> io::Result<bool> {
    let mut information = std::mem::MaybeUninit::<libc::proc_bsdinfo>::zeroed();
    let size = std::mem::size_of::<libc::proc_bsdinfo>();
    let buffer_size = libc::c_int::try_from(size)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "process state is too large"))?;
    // SAFETY: `information` is writable storage for exactly one
    // `proc_bsdinfo`; PROC_PIDTBSDINFO writes at most `buffer_size` bytes.
    let read = unsafe {
        libc::proc_pidinfo(
            pid,
            libc::PROC_PIDTBSDINFO,
            DARWIN_INCLUDE_ZOMBIES,
            information.as_mut_ptr().cast(),
            buffer_size,
        )
    };
    if read != buffer_size {
        let error = io::Error::last_os_error();
        return Err(
            if read <= 0 && error.raw_os_error().is_some_and(|code| code != 0) {
                error
            } else {
                io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    format!("process state returned {read} of {buffer_size} bytes"),
                )
            },
        );
    }
    // SAFETY: a full proc_bsdinfo was initialized when proc_pidinfo returned
    // its exact size.
    let information = unsafe { information.assume_init() };
    if information.pbi_pid != pid as u32 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "process state described a different PID",
        ));
    }
    Ok(matches!(
        darwin_process_snapshot_state(information.pbi_status, information.pbi_flags),
        DarwinProcessState::Zombie
    ))
}

#[cfg(target_os = "macos")]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DarwinProcessState {
    Live,
    Exiting,
    Zombie,
}

#[cfg(target_os = "macos")]
fn darwin_process_snapshot_state(status: u32, flags: u32) -> DarwinProcessState {
    if status == libc::SZOMB {
        DarwinProcessState::Zombie
    } else if flags & DARWIN_PROC_FLAG_INEXIT != 0 {
        DarwinProcessState::Exiting
    } else {
        DarwinProcessState::Live
    }
}

/// Observes a completed Unix child without releasing its process identity,
/// stops any descendants still in its owned group, and only then reaps it.
///
/// `Child::try_wait` reaps immediately. Sending a signal to `-child.id()`
/// after that would address a reusable number rather than the group Runyte
/// created. Linux and other Unix targets use `waitid(WNOWAIT)` to keep the
/// exited leader waitable. Darwin level-queries both the live and zombie
/// process tables because its `waitid` implementation does not provide a
/// reliable nonblocking completion boundary here. Both keep the PID and
/// process-group identity anchored through cleanup.
#[cfg(all(unix, not(target_os = "macos")))]
fn try_finish_child(
    child: &mut std::process::Child,
    _observer: &ChildExitObserver,
) -> io::Result<Option<std::process::ExitStatus>> {
    let pid = i32::try_from(child.id())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "child PID does not fit pid_t"))?;
    let mut information = std::mem::MaybeUninit::<libc::siginfo_t>::zeroed();
    // SAFETY: `information` points to writable storage for `siginfo_t`; P_PID
    // limits the observation to this live Child handle, and WNOWAIT leaves the
    // reported exit available for `Child::wait` below.
    let result = unsafe {
        libc::waitid(
            libc::P_PID,
            pid as libc::id_t,
            information.as_mut_ptr(),
            libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
        )
    };
    if result == -1 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: successful waitid initializes the supplied structure. With
    // WNOHANG, si_pid is zero when the child has not exited yet.
    let information = unsafe { information.assume_init() };
    if unsafe { information.si_pid() } == 0 {
        return Ok(None);
    }
    stop_child_group(child);
    child.wait().map(Some)
}

#[cfg(target_os = "macos")]
fn try_finish_child(
    child: &mut std::process::Child,
    observer: &ChildExitObserver,
) -> io::Result<Option<std::process::ExitStatus>> {
    if !observer.exited()? {
        return Ok(None);
    }
    stop_child_group(child);
    child.wait().map(Some)
}

#[cfg(not(unix))]
fn try_finish_child(
    child: &mut std::process::Child,
    _observer: &ChildExitObserver,
) -> io::Result<Option<std::process::ExitStatus>> {
    child.try_wait()
}

#[cfg(unix)]
fn stop_child_group(child: &std::process::Child) {
    if let Ok(pid) = i32::try_from(child.id()) {
        // SAFETY: Git children create a process group whose ID is the child's
        // PID in `spawn`; a negative PID addresses that group.
        unsafe {
            libc::kill(-pid, libc::SIGKILL);
        }
    }
}

/// Stops the command and, on Unix, every helper it started.
fn stop_child_tree(child: &mut std::process::Child) {
    #[cfg(unix)]
    stop_child_group(child);
    let _ = child.kill();
    let _ = child.wait();
}

/// Gives pipe workers one scheduling turn after Git exits, then stops the
/// owned process group and gives the readers one final bounded grace period.
///
/// A detached hook or helper is still part of the service-owned process tree;
/// it must not keep a repository lock or worker alive after the top-level Git
/// command has completed. A descendant that escaped the process group can
/// keep its detached reader thread until it closes the pipe, but never the
/// caller waiting on an unbounded join.
#[cfg(not(unix))]
fn finish_readers_or_stop(
    child: &mut std::process::Child,
    mut readers_finished: impl FnMut() -> bool,
) -> bool {
    let deadline = std::time::Instant::now() + NETWORK_POLL_INTERVAL;
    while !readers_finished() && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    if readers_finished() {
        return true;
    }
    stop_child_tree(child);
    let deadline = std::time::Instant::now() + NETWORK_POLL_INTERVAL;
    while !readers_finished() && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    readers_finished()
}

fn unclosed_output_error(directory: &Path) -> GitError {
    GitError::Io {
        action: "finish reading Git output in",
        path: directory.to_path_buf(),
        detail: "a Git descendant kept inherited pipes open after Git exited".to_owned(),
    }
}

fn has_git_marker(start: &Path) -> Result<bool> {
    let start = start.canonicalize().map_err(|error| GitError::Io {
        action: "inspect repository ancestry from",
        path: start.to_path_buf(),
        detail: error.to_string(),
    })?;
    let temporary = std::env::temp_dir().canonicalize().ok();
    has_git_marker_in(start.ancestors(), |directory| {
        directory != start
            && (temporary.as_deref() == Some(directory) || is_shared_scratch_directory(directory))
    })
}

fn has_git_marker_in<'a>(
    directories: impl IntoIterator<Item = &'a Path>,
    mut stop_before: impl FnMut(&Path) -> bool,
) -> Result<bool> {
    for directory in directories {
        if stop_before(directory) {
            break;
        }
        let marker = directory.join(".git");
        match std::fs::symlink_metadata(&marker) {
            Ok(metadata) if metadata.file_type().is_dir() => {
                let head = marker.join("HEAD");
                let metadata = std::fs::symlink_metadata(&head).map_err(|error| GitError::Io {
                    action: "inspect repository marker at",
                    path: marker.clone(),
                    detail: format!("HEAD is unavailable: {error}"),
                })?;
                if !metadata.file_type().is_file() {
                    return Err(GitError::Io {
                        action: "inspect repository marker at",
                        path: marker,
                        detail: "HEAD is not a regular file".to_owned(),
                    });
                }
                return Ok(true);
            }
            Ok(metadata) if metadata.file_type().is_file() => {
                let file = std::fs::File::open(&marker).map_err(|error| GitError::Io {
                    action: "inspect repository marker at",
                    path: marker.clone(),
                    detail: error.to_string(),
                })?;
                let mut content = Vec::new();
                file.take(MAX_GIT_MARKER_BYTES as u64 + 1)
                    .read_to_end(&mut content)
                    .map_err(|error| GitError::Io {
                        action: "inspect repository marker at",
                        path: marker.clone(),
                        detail: error.to_string(),
                    })?;
                if content.len() > MAX_GIT_MARKER_BYTES {
                    return Err(GitError::Io {
                        action: "inspect repository marker at",
                        path: marker,
                        detail: format!("the gitdir file exceeds {MAX_GIT_MARKER_BYTES} bytes"),
                    });
                }
                let first_line = content
                    .split(|byte| *byte == b'\n')
                    .next()
                    .unwrap_or_default();
                let target = first_line
                    .strip_prefix(b"gitdir: ")
                    .unwrap_or_default()
                    .iter()
                    .copied()
                    .filter(|byte| !byte.is_ascii_whitespace())
                    .collect::<Vec<_>>();
                if target.is_empty() {
                    return Err(GitError::Io {
                        action: "inspect repository marker at",
                        path: marker,
                        detail: "the gitdir file has no target".to_owned(),
                    });
                }
                return Ok(true);
            }
            Ok(_) => {
                return Err(GitError::Io {
                    action: "inspect repository marker at",
                    path: marker,
                    detail: "the marker is not a file or directory".to_owned(),
                });
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(GitError::Io {
                    action: "inspect repository marker at",
                    path: marker,
                    detail: error.to_string(),
                });
            }
        }
    }
    Ok(false)
}

fn is_shared_scratch_directory(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        std::fs::metadata(path).is_ok_and(|metadata| {
            metadata.is_dir() && metadata.permissions().mode() & 0o1002 == 0o1002
        })
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        false
    }
}

impl GitCliProvider {
    fn discover_with_marker_probe(
        &self,
        start: &Path,
        marker_probe: impl FnOnce(&Path) -> Result<bool>,
    ) -> Result<Option<Repository>> {
        if !start.exists() {
            return Err(GitError::Io {
                action: "look for a repository at",
                path: start.to_path_buf(),
                detail: "the path does not exist".to_owned(),
            });
        }
        // A missing marker is the ordinary, stable absence case. Once a
        // repository marker exists, Git is the authority for its layout and
        // every failure is material: converting a transient reader or
        // permissions failure into `None` would cache a false absence for the
        // lifetime of this workspace.
        if !marker_probe(start)? {
            return Ok(None);
        }
        let toplevel = self.run_text(start, &["rev-parse", "--show-toplevel"])?;
        if toplevel.is_empty() {
            return Err(GitError::Malformed {
                command: self.describe(&["rev-parse", "--show-toplevel"]),
                detail: "Git reported an empty repository root after a repository marker was found"
                    .to_owned(),
            });
        }
        let workdir = PathBuf::from(toplevel);
        let git_dir_text = self.run_text(&workdir, &["rev-parse", "--git-dir"])?;
        if git_dir_text.is_empty() {
            return Err(GitError::Malformed {
                command: self.describe(&["rev-parse", "--git-dir"]),
                detail: "Git reported an empty repository metadata directory".to_owned(),
            });
        }
        let git_dir = PathBuf::from(git_dir_text);
        let git_dir = if git_dir.is_absolute() {
            git_dir
        } else {
            workdir.join(git_dir)
        };
        let git_dir = git_dir.canonicalize().unwrap_or(git_dir);
        let common_text = self.run_text(&workdir, &["rev-parse", "--git-common-dir"])?;
        if common_text.is_empty() {
            return Err(GitError::Malformed {
                command: self.describe(&["rev-parse", "--git-common-dir"]),
                detail: "Git reported an empty common metadata directory".to_owned(),
            });
        }
        let common = PathBuf::from(common_text);
        let common = if common.is_absolute() {
            common
        } else {
            workdir.join(common)
        };
        let common = common.canonicalize().unwrap_or(common);
        Ok(Some(Repository::with_git_dirs(workdir, git_dir, common)))
    }

    /// Reviews a branch deletion, optionally tolerating the one checkout a
    /// cascade is removing on the way to it.
    ///
    /// Git refuses to delete a branch that is checked out, so an ordinary
    /// review refuses one too rather than preparing a deletion that cannot be
    /// applied. `allowed_checkout` is the exception a cascade needs: that
    /// checkout is being removed by the same confirmed action, and is gone
    /// before anything reaches `delete_branch_guarded`, whose own re-review
    /// carries no exception at all.
    fn review_branch_deletion(
        &self,
        repository: &Repository,
        branch: &str,
        allowed_checkout: Option<&Path>,
    ) -> Result<BranchDeletionPlan> {
        let branches = self.branches(repository)?;
        let target = branches
            .iter()
            .find(|candidate| candidate.name == branch)
            .ok_or_else(|| GitError::Failed {
                command: "git branch".to_owned(),
                code: None,
                stderr: format!("`{branch}` is not a local branch"),
            })?;
        let unexpected_checkout = target
            .checkouts
            .iter()
            .any(|checkout| Some(checkout.as_path()) != allowed_checkout);
        if target.current || unexpected_checkout {
            return Err(GitError::Failed {
                command: "git branch".to_owned(),
                code: None,
                stderr: format!("`{branch}` is checked out in a worktree"),
            });
        }
        let reference = format!("refs/heads/{branch}");
        let tip = self.run_text(
            repository.workdir(),
            &["rev-parse", "--verify", reference.as_str()],
        )?;
        let tip = tip.trim().to_owned();
        let containing = self.run_text(
            repository.workdir(),
            &[
                "for-each-ref",
                "--format=%(refname:short)",
                &format!("--contains={tip}"),
                "refs/heads",
            ],
        )?;
        let mut retaining_branches = containing
            .lines()
            .filter(|candidate| *candidate != branch)
            .map(str::to_owned)
            .collect::<Vec<_>>();
        retaining_branches.sort();
        let upstream_retains = target.upstream.as_ref().is_some_and(|upstream| {
            upstream
                .divergence
                .is_some_and(|divergence| divergence.ahead == 0)
        });
        Ok(BranchDeletionPlan {
            branch: branch.to_owned(),
            tip,
            upstream: target.upstream.clone(),
            required_authorization: if upstream_retains || !retaining_branches.is_empty() {
                DeletionAuthorization::Enter
            } else {
                DeletionAuthorization::Typed
            },
            retaining_branches,
        })
    }
}

impl GitProvider for GitCliProvider {
    fn discover(&self, start: &Path) -> Result<Option<Repository>> {
        self.discover_with_marker_probe(start, has_git_marker)
    }

    fn status(&self, repository: &Repository) -> Result<RepositoryStatus> {
        let arguments = ["status", "--porcelain=v2", "--branch", "-z"];
        let output = self.run(repository.workdir(), &arguments)?;
        status::parse(&output).map_err(|detail| GitError::Malformed {
            command: self.describe(&arguments),
            detail,
        })
    }

    fn status_stats(
        &self,
        repository: &Repository,
        status: &RepositoryStatus,
    ) -> Result<StatusStats> {
        let mut stats = StatusStats::default();
        for scope in [DiffScope::Staged, DiffScope::Unstaged] {
            stats.extend(scope, self.numstat(repository, scope)?);
        }
        let mut budget = MAX_UNTRACKED_STAT_BUDGET;
        for file in status.files.iter().filter(|file| file.is_untracked()) {
            if budget == 0 {
                break;
            }
            let path = repository.workdir().join(&file.path);
            // An untracked directory is one row standing for a whole tree Git
            // never looked into. Counting it would mean walking that tree,
            // which is exactly the work `status` avoided by collapsing it.
            if path.is_dir() {
                continue;
            }
            let limit = MAX_UNTRACKED_STAT_BYTES.min(budget);
            let Some(content) = read_bounded_file(repository.workdir(), &path, limit) else {
                continue;
            };
            budget -= content.len();
            if let Some(counted) = count_new_lines(&content) {
                stats.insert(DiffScope::Unstaged, file.path.clone(), counted);
            }
        }
        Ok(stats)
    }

    fn head_oid(&self, repository: &Repository) -> Result<Option<String>> {
        self.run_text(repository.workdir(), &["rev-parse", "--verify", "HEAD"])
            .map(Some)
    }

    fn branches(&self, repository: &Repository) -> Result<Vec<Branch>> {
        // A unit separator between the fields: a ref name cannot contain a
        // control character, and neither can the upstream name, so nothing a
        // branch is called can be mistaken for the boundary between fields.
        let arguments = [
            "for-each-ref",
            "--format=%(refname:short)%1f%(upstream:short)%1f%(upstream:remotename)%1f\
             %(upstream:remoteref)%1f%(upstream:track)",
            "refs/heads",
        ];
        let output = self.run_text(repository.workdir(), &arguments)?;
        let status = self.status(repository)?;
        let current = match &status.head {
            Head::Branch(name) | Head::Unborn(name) => Some(name.as_str()),
            Head::Detached(_) => None,
        };
        // One extra ref walk rather than an is-ancestor call per branch. This
        // only fails where `HEAD` names nothing yet, and an unborn branch has
        // nothing below it to have merged anywhere.
        let merged = self
            .run_text(
                repository.workdir(),
                &[
                    "for-each-ref",
                    "--format=%(refname:short)",
                    "--merged=HEAD",
                    "refs/heads",
                ],
            )
            .unwrap_or_default();
        let merged = merged.lines().collect::<Vec<_>>();
        let worktrees = self.worktrees(repository)?;
        let checkouts_for = |name: &str| {
            let reference = format!("refs/heads/{name}");
            worktrees
                .iter()
                .filter(|worktree| worktree.branch.as_deref() == Some(reference.as_str()))
                .map(|worktree| worktree.path.clone())
                .collect::<Vec<_>>()
        };
        let mut branches = output
            .lines()
            .filter(|line| !line.is_empty())
            .map(|line| {
                let mut fields = line.split('\u{1f}');
                let name = fields.next().unwrap_or_default();
                let upstream = fields.next().unwrap_or_default();
                let remote = fields.next().unwrap_or_default();
                let reference = fields.next().unwrap_or_default();
                let track = fields.next().unwrap_or_default();
                Branch {
                    name: name.to_owned(),
                    current: current == Some(name),
                    checkouts: checkouts_for(name),
                    upstream: parse_upstream(upstream, remote, reference, track),
                    merged: merged.contains(&name),
                }
            })
            .collect::<Vec<_>>();
        if let Some(current) = current
            && !branches.iter().any(|branch| branch.name == current)
        {
            let mut branch = Branch::new(current, true);
            branch.checkouts = checkouts_for(current);
            branches.push(branch);
        }
        branches.sort_by(|left, right| left.name.cmp(&right.name));
        Ok(branches)
    }

    fn worktrees(&self, repository: &Repository) -> Result<Vec<Worktree>> {
        let arguments = ["worktree", "list", "--porcelain", "-z"];
        let output = self.run(repository.workdir(), &arguments)?;
        parse_worktree_porcelain(repository, &output)
    }

    fn create_worktree(&self, repository: &Repository, request: &WorktreeCreate) -> Result<()> {
        if request.start.starts_with('-')
            || request
                .new_branch
                .as_deref()
                .is_some_and(|branch| branch.starts_with('-'))
        {
            return Err(GitError::Failed {
                command: "git worktree add".to_owned(),
                code: None,
                stderr: "branch names beginning with `-` are refused".to_owned(),
            });
        }
        let mut arguments = vec![OsString::from("worktree"), OsString::from("add")];
        if let Some(branch) = &request.new_branch {
            arguments.push(OsString::from("-b"));
            arguments.push(OsString::from(branch));
        } else if !self
            .branches(repository)?
            .iter()
            .any(|branch| branch.name == request.start)
        {
            return Err(GitError::Failed {
                command: "git worktree add".to_owned(),
                code: None,
                stderr: format!("`{}` is not a local branch", request.start),
            });
        }
        arguments.push(OsString::from("--"));
        arguments.push(request.destination.as_os_str().to_owned());
        arguments.push(OsString::from(&request.start));
        self.run(repository.workdir(), &arguments).map(|_| ())
    }

    fn remove_worktree(&self, repository: &Repository, path: &Path) -> Result<()> {
        self.run(
            repository.workdir(),
            &[
                OsStr::new("worktree"),
                OsStr::new("remove"),
                OsStr::new("--"),
                path.as_os_str(),
            ],
        )
        .map(|_| ())
    }

    fn prepare_worktree_removal(
        &self,
        repository: &Repository,
        path: &Path,
    ) -> Result<WorktreeRemovalPlan> {
        let worktree = self
            .worktrees(repository)?
            .into_iter()
            .find(|worktree| worktree.path == path)
            .ok_or_else(|| GitError::Failed {
                command: "git worktree list".to_owned(),
                code: None,
                stderr: format!("{} is no longer a registered worktree", path.display()),
            })?;
        let target_repository =
            Repository::with_common_dir(&worktree.path, repository.common_dir());
        let status = self.status(&target_repository)?;
        if !status.files.is_empty() {
            return Err(GitError::Failed {
                command: "git worktree remove".to_owned(),
                code: None,
                stderr: format!(
                    "worktree {} has uncommitted changes ({} file{}); commit, stash, or discard them first",
                    worktree.path.display(),
                    status.files.len(),
                    if status.files.len() == 1 { "" } else { "s" }
                ),
            });
        }
        let branch = worktree.branch.as_deref().map(|branch| {
            branch
                .strip_prefix("refs/heads/")
                .unwrap_or(branch)
                .to_owned()
        });
        let upstream = if let Some(name) = branch.as_deref() {
            self.branches(repository)?
                .into_iter()
                .find(|candidate| candidate.name == name)
                .ok_or_else(|| GitError::Failed {
                    command: "git branch --format".to_owned(),
                    code: None,
                    stderr: format!(
                        "worktree {} names branch {name}, but that branch could not be inspected",
                        worktree.path.display()
                    ),
                })?
                .upstream
        } else {
            None
        };
        let detached_retained = if branch.is_none() {
            match worktree.head.as_deref() {
                Some(head) => !self
                    .run_text(
                        repository.workdir(),
                        &[
                            "for-each-ref",
                            "--format=%(refname)",
                            &format!("--contains={head}"),
                            "refs/heads",
                            "refs/remotes",
                        ],
                    )?
                    .trim()
                    .is_empty(),
                None => true,
            }
        } else {
            false
        };
        let tracked_unpublished = upstream.as_ref().is_some_and(|upstream| {
            upstream
                .divergence
                .is_none_or(|divergence| divergence.ahead > 0)
        });
        let required_authorization =
            if tracked_unpublished || (branch.is_none() && !detached_retained) {
                DeletionAuthorization::Typed
            } else {
                DeletionAuthorization::Enter
            };
        Ok(WorktreeRemovalPlan {
            path: worktree.path,
            head: worktree.head,
            branch,
            upstream,
            detached_retained,
            required_authorization,
        })
    }

    fn remove_worktree_guarded(
        &self,
        repository: &Repository,
        plan: &WorktreeRemovalPlan,
        authorization: DeletionAuthorization,
    ) -> Result<()> {
        let current = self.prepare_worktree_removal(repository, &plan.path)?;
        if current != *plan {
            return Err(GitError::Failed {
                command: "git worktree remove".to_owned(),
                code: None,
                stderr: "the worktree changed after it was reviewed; review the removal again"
                    .to_owned(),
            });
        }
        if authorization < current.required_authorization {
            return Err(GitError::Failed {
                command: "git worktree remove".to_owned(),
                code: None,
                stderr: "this worktree has unpublished history and needs typed confirmation"
                    .to_owned(),
            });
        }
        self.remove_worktree(repository, &plan.path)
    }

    fn log_page(&self, repository: &Repository, request: &LogRequest) -> Result<LogPage> {
        if request.limit == 0 || request.limit > MAX_LOG_PAGE_SIZE {
            return Err(GitError::Failed {
                command: "git log".to_owned(),
                code: None,
                stderr: format!("history page size must be between 1 and {MAX_LOG_PAGE_SIZE}"),
            });
        }
        let start = match &request.cursor {
            Some(LogCursor { boundary }) => {
                if !valid_object_id(boundary) {
                    return Err(GitError::Malformed {
                        command: "git log".to_owned(),
                        detail: "history cursor is not a full object id".to_owned(),
                    });
                }
                format!("{boundary}^@")
            }
            None => "HEAD".to_owned(),
        };
        let total_output =
            self.run_read_bounded(repository.workdir(), &["rev-list", "--count", "HEAD"], 64)?;
        let total_commits = std::str::from_utf8(&total_output)
            .ok()
            .and_then(|output| output.trim().parse::<usize>().ok())
            .ok_or_else(|| GitError::Malformed {
                command: "git rev-list".to_owned(),
                detail: "reachable commit count is not an integer".to_owned(),
            })?;
        let total_pages = total_commits.div_ceil(request.limit).max(1);
        let count = request.limit.saturating_add(1).to_string();
        let arguments = vec![
            OsString::from("log"),
            OsString::from("-z"),
            OsString::from("--topo-order"),
            OsString::from("--date-order"),
            OsString::from("--abbrev=12"),
            OsString::from(format!("--max-count={count}")),
            OsString::from("--format=%H%x00%h%x00%P%x00%an%x00%at%x00%as%x00%s%x00%D"),
            OsString::from(start),
        ];
        let mut commits = parse_log(&self.run_read_bounded(
            repository.workdir(),
            &arguments,
            self.max_output_bytes,
        )?)?;
        let has_more = commits.len() > request.limit;
        commits.truncate(request.limit);
        let next = has_more
            .then(|| commits.last())
            .flatten()
            .map(|commit| LogCursor {
                boundary: commit.oid.clone(),
            });
        Ok(LogPage {
            commits,
            next,
            total_pages,
        })
    }

    fn search_commits(&self, repository: &Repository) -> Result<CommitSearchResult> {
        let count = MAX_COMMIT_SEARCH_RESULTS.saturating_add(1).to_string();
        let arguments = vec![
            OsString::from("log"),
            OsString::from("-z"),
            OsString::from("--topo-order"),
            OsString::from("--date-order"),
            OsString::from("--abbrev=12"),
            OsString::from(format!("--max-count={count}")),
            OsString::from("--format=%H%x00%h%x00%P%x00%an%x00%at%x00%as%x00%s%x00%D%x00%B"),
            OsString::from("HEAD"),
        ];
        let mut commits = parse_commit_search(&self.run_read_bounded(
            repository.workdir(),
            &arguments,
            self.max_output_bytes,
        )?)?;
        let limited = commits.len() > MAX_COMMIT_SEARCH_RESULTS;
        commits.truncate(MAX_COMMIT_SEARCH_RESULTS);
        Ok(CommitSearchResult { commits, limited })
    }

    fn history_contains(&self, repository: &Repository, oid: &str) -> Result<bool> {
        if !valid_object_id(oid) {
            return Err(GitError::Malformed {
                command: "git merge-base --is-ancestor".to_owned(),
                detail: "history anchor is not a full object id".to_owned(),
            });
        }
        match self.run_read_bounded(
            repository.workdir(),
            &["merge-base", "--is-ancestor", oid, "HEAD"],
            1,
        ) {
            Ok(_) => Ok(true),
            Err(GitError::Failed { code: Some(1), .. }) => Ok(false),
            Err(error) => Err(error),
        }
    }

    fn stashes(&self, repository: &Repository) -> Result<Vec<StashEntry>> {
        let arguments = [
            "stash",
            "list",
            "-z",
            "--max-count=201",
            "--format=%H%x00%gd%x00%gs",
        ];
        parse_stashes(&self.run_read_bounded(
            repository.workdir(),
            &arguments,
            self.max_output_bytes,
        )?)
    }

    fn mutate_stash(&self, repository: &Repository, mutation: &StashMutation) -> Result<String> {
        match mutation {
            StashMutation::Create { name, scope } => {
                if name.is_empty() {
                    return Err(GitError::Failed {
                        command: "git stash push".to_owned(),
                        code: None,
                        stderr: "a named stash needs a non-empty name".to_owned(),
                    });
                }
                let mut arguments = vec![OsString::from("stash"), OsString::from("push")];
                match scope {
                    StashScope::TrackedWorktree => arguments.push(OsString::from("--keep-index")),
                    StashScope::TrackedWorktreeAndIndex => {}
                    StashScope::TrackedAndUntracked => {
                        arguments.push(OsString::from("--include-untracked"))
                    }
                }
                arguments.extend([OsString::from("-m"), OsString::from(name)]);
                self.run_text(repository.workdir(), &arguments)
            }
            StashMutation::Apply { oid } => {
                if !valid_object_id(oid) {
                    return Err(GitError::Malformed {
                        command: "git stash apply".to_owned(),
                        detail: "stash identity is not a full object ID".to_owned(),
                    });
                }
                let entry = self
                    .stashes(repository)?
                    .into_iter()
                    .find(|entry| entry.oid == *oid)
                    .ok_or_else(|| GitError::Failed {
                        command: "git stash apply".to_owned(),
                        code: None,
                        stderr: "the selected stash no longer exists; refresh the list".to_owned(),
                    })?;
                self.run_text(repository.workdir(), &["stash", "apply", &entry.oid])
                    .map_err(stash_apply_error)
            }
            StashMutation::Drop { oid } => {
                let entry = self
                    .stashes(repository)?
                    .into_iter()
                    .find(|entry| entry.oid == *oid)
                    .ok_or_else(|| GitError::Failed {
                        command: "git stash drop".to_owned(),
                        code: None,
                        stderr: "the selected stash no longer exists; refresh the list".to_owned(),
                    })?;
                self.run_text(repository.workdir(), &["stash", "drop", &entry.selector])
            }
        }
    }

    fn repository_fingerprint(&self, repository: &Repository) -> Result<RepositoryFingerprint> {
        let status = self.status(repository)?;
        let head = if matches!(status.head, Head::Unborn(_)) {
            None
        } else {
            self.head_oid(repository)?
        };
        let index = self.run_read_bounded(
            repository.workdir(),
            &["ls-files", "--stage", "-z"],
            self.max_output_bytes,
        )?;
        Ok(RepositoryFingerprint {
            head,
            index: crate::hash::sha256_hex(&index),
        })
    }

    fn apply_partial(&self, repository: &Repository, request: &PartialStageRequest) -> Result<()> {
        if request.repository != repository.common_dir()
            || !valid_fingerprint(&request.fingerprint)
            || request.patch.len() > MAX_PATCH_BYTES
            || request.buffer.is_some() != request.guard.is_some()
        {
            return Err(GitError::Malformed {
                command: "partial staging".to_owned(),
                detail: "partial-stage request identity or bounds are invalid".to_owned(),
            });
        }
        if request
            .guard
            .as_ref()
            .is_some_and(|guard| !guard.is_valid())
        {
            return stale_partial();
        }
        let parsed = super::parse_hunks(&request.patch)?;
        if parsed.len() != 1 || parsed[0].identity != request.hunk {
            return Err(GitError::Malformed {
                command: "partial staging".to_owned(),
                detail: "the exact patch must contain only its identified hunk".to_owned(),
            });
        }
        self.relative(repository, &request.path)?;
        let current_diff = self.diff(repository, request.scope, Some(&request.path))?;
        let current_hunks = match super::parse_hunks(current_diff.as_bytes()) {
            Ok(hunks) => hunks,
            Err(GitError::Failed { .. }) => return stale_partial(),
            Err(error) => return Err(error),
        };
        if !current_hunks
            .iter()
            .any(|hunk| hunk.identity == request.hunk && hunk.patch == request.patch)
        {
            return stale_partial();
        }
        let actual = self.repository_fingerprint(repository)?;
        let disk = bounded_file_sha256(repository, &request.path)?;
        if actual != request.fingerprint || disk != request.disk_sha256 {
            return stale_partial();
        }
        let mut arguments = vec![OsString::from("apply"), OsString::from("--cached")];
        if request.scope == super::DiffScope::Staged {
            arguments.push(OsString::from("--reverse"));
        }
        let mut check = arguments.clone();
        check.push(OsString::from("--check"));
        self.run_with_input_bounded(
            repository.workdir(),
            &check,
            &request.patch,
            self.max_output_bytes,
        )?;
        if request
            .guard
            .as_ref()
            .is_some_and(|guard| !guard.is_valid())
        {
            return stale_partial();
        }
        if self.repository_fingerprint(repository)? != request.fingerprint
            || bounded_file_sha256(repository, &request.path)? != request.disk_sha256
        {
            return stale_partial();
        }
        if request
            .guard
            .as_ref()
            .is_some_and(|guard| !guard.is_valid())
        {
            return stale_partial();
        }
        self.run_with_input_bounded(
            repository.workdir(),
            &arguments,
            &request.patch,
            self.max_output_bytes,
        )
        .map(|_| ())
    }

    fn prepare_partial(
        &self,
        repository: &Repository,
        selection: &PartialStageSelection,
    ) -> Result<PartialStageRequest> {
        if selection.buffer.is_some() != selection.guard.is_some() {
            return Err(GitError::Malformed {
                command: "partial staging".to_owned(),
                detail: "a live buffer revision and guard must be paired".to_owned(),
            });
        }
        let fingerprint = self.repository_fingerprint(repository)?;
        let disk_sha256 = bounded_file_sha256(repository, &selection.path)?;
        let relative = self.relative(repository, &selection.path)?;
        let status = self.status(repository)?;
        let file = status
            .files
            .iter()
            .find(|file| file.path == relative)
            .ok_or_else(|| GitError::Failed {
                command: "partial staging".to_owned(),
                code: None,
                stderr: "the path has no Git changes".to_owned(),
            })?;
        if file.is_conflicted() || file.is_untracked() || file.original_path.is_some() {
            return Err(GitError::Failed {
                command: "partial staging".to_owned(),
                code: None,
                stderr:
                    "conflicts, untracked files, and renames require Lazygit or a whole-file action"
                        .to_owned(),
            });
        }
        let diff = self.diff(repository, selection.scope, Some(&selection.path))?;
        let hunks = super::parse_hunks(diff.as_bytes())?;
        let hunk = if let Some(identity) = &selection.hunk {
            hunks
                .into_iter()
                .find(|hunk| &hunk.identity == identity)
                .ok_or_else(|| GitError::Failed {
                    command: "partial staging".to_owned(),
                    code: None,
                    stderr: "stale hunk: the diff changed; refresh and retry".to_owned(),
                })?
        } else if let Some((first, last)) = selection.lines {
            let mut matching = hunks
                .into_iter()
                .filter(|hunk| hunk.intersects_new_range(first, last));
            let hunk = matching.next().ok_or_else(|| GitError::Failed {
                command: "selected-line staging".to_owned(),
                code: None,
                stderr: "the selection contains no stageable added or modified lines".to_owned(),
            })?;
            if matching.next().is_some() {
                return Err(GitError::Failed {
                    command: "selected-line staging".to_owned(),
                    code: None,
                    stderr: "the selection crosses multiple hunks; stage each hunk or use Lazygit"
                        .to_owned(),
                });
            }
            let patch = super::select_lines(&hunk, first, last)?;
            super::PatchHunk { patch, ..hunk }
        } else {
            return Err(GitError::Malformed {
                command: "partial staging".to_owned(),
                detail: "request has neither a hunk nor selected lines".to_owned(),
            });
        };
        if self.repository_fingerprint(repository)? != fingerprint
            || bounded_file_sha256(repository, &selection.path)? != disk_sha256
            || selection
                .guard
                .as_ref()
                .is_some_and(|guard| !guard.is_valid())
        {
            return stale_partial();
        }
        Ok(PartialStageRequest {
            repository: repository.common_dir().to_path_buf(),
            fingerprint,
            path: selection.path.clone(),
            disk_sha256,
            buffer: selection.buffer,
            guard: selection.guard.clone(),
            scope: selection.scope,
            hunk: hunk.identity,
            patch: hunk.patch,
        })
    }

    fn commit_detail(&self, repository: &Repository, oid: &str) -> Result<CommitDetail> {
        if !valid_object_id(oid) {
            return Err(GitError::Malformed {
                command: "git show".to_owned(),
                detail: "commit detail identity is not a full object id".to_owned(),
            });
        }
        let summary_arguments = [
            OsString::from("log"),
            OsString::from("-z"),
            OsString::from("-1"),
            OsString::from("--abbrev=12"),
            OsString::from("--format=%H%x00%h%x00%P%x00%an%x00%at%x00%as%x00%s%x00%D"),
            OsString::from(oid),
        ];
        let mut summaries = parse_log(&self.run_read_bounded(
            repository.workdir(),
            &summary_arguments,
            self.max_output_bytes,
        )?)?;
        let summary = summaries.pop().ok_or_else(|| GitError::Malformed {
            command: "git log".to_owned(),
            detail: "commit detail returned no metadata".to_owned(),
        })?;
        let body_arguments = ["show", "-s", "--format=%B", oid];
        let body = String::from_utf8_lossy(&self.run_read_bounded(
            repository.workdir(),
            &body_arguments,
            self.max_output_bytes,
        )?)
        .into_owned();
        let patch_arguments = [
            "show",
            "--format=",
            "--patch",
            "--no-ext-diff",
            "--no-color",
            oid,
        ];
        let patch = String::from_utf8_lossy(&self.run_read_bounded(
            repository.workdir(),
            &patch_arguments,
            self.patch_output_bytes(),
        )?)
        .into_owned();
        Ok(CommitDetail {
            summary,
            body,
            patch,
        })
    }

    fn blame(&self, repository: &Repository, request: &BlameRequest) -> Result<Vec<BlameLine>> {
        if request.content.len() > MAX_BLAME_INPUT_BYTES {
            return Err(GitError::TooLarge {
                command: "git blame --contents".to_owned(),
                limit: MAX_BLAME_INPUT_BYTES,
            });
        }
        if request.content.as_bytes().contains(&0) {
            return Err(GitError::Failed {
                command: "git blame --contents".to_owned(),
                code: None,
                stderr: "binary buffers cannot be blamed".to_owned(),
            });
        }
        if request.lines.is_none() && request.content.lines().count() > MAX_BLAME_LINES {
            return Err(GitError::Failed {
                command: "git blame --contents".to_owned(),
                code: None,
                stderr: format!("full-file blame is limited to {MAX_BLAME_LINES} lines"),
            });
        }
        let relative = self
            .relative(repository, &request.path)
            .map_err(|_| GitError::Failed {
                command: "git blame".to_owned(),
                code: None,
                stderr: "the buffer is outside this working tree".to_owned(),
            })?;
        let status = self.status(repository)?;
        let blame_path = status
            .files
            .iter()
            .find(|file| file.path == relative)
            .and_then(|file| file.original_path.as_deref())
            .unwrap_or(relative);
        let mut arguments = vec![
            OsString::from("--literal-pathspecs"),
            OsString::from("blame"),
            OsString::from("--line-porcelain"),
            OsString::from("--contents"),
            OsString::from("-"),
        ];
        if let Some((start, end)) = request.lines {
            if start == 0 || end < start {
                return Err(GitError::Malformed {
                    command: "git blame".to_owned(),
                    detail: "blame line window is invalid".to_owned(),
                });
            }
            arguments.push(OsString::from("-L"));
            arguments.push(OsString::from(format!("{start},{end}")));
        }
        arguments.push(OsString::from("--"));
        arguments.push(blame_path.as_os_str().to_owned());
        let output = self.run_with_input_bounded(
            repository.workdir(),
            &arguments,
            request.content.as_bytes(),
            self.max_output_bytes,
        )?;
        parse_blame(&output)
    }

    fn checkout_branch(&self, repository: &Repository, branch: &str) -> Result<()> {
        let status = self.status(repository)?;
        if !status.files.is_empty() {
            return Err(GitError::DirtyWorktree {
                files: status.files.len(),
            });
        }
        if branch.starts_with('-')
            || !self
                .branches(repository)?
                .iter()
                .any(|candidate| candidate.name == branch)
        {
            return Err(GitError::Failed {
                command: "git checkout".to_owned(),
                code: None,
                stderr: format!("`{branch}` is not a local branch"),
            });
        }
        self.run(
            repository.workdir(),
            &[
                OsStr::new("checkout"),
                OsStr::new("--quiet"),
                OsStr::new(branch),
            ],
        )
        .map(|_| ())
    }

    fn create_branch(
        &self,
        repository: &Repository,
        branch: &str,
        start_point: &str,
    ) -> Result<()> {
        let status = self.status(repository)?;
        if !status.files.is_empty() {
            return Err(GitError::DirtyWorktree {
                files: status.files.len(),
            });
        }
        if branch.starts_with('-') {
            return Err(GitError::Failed {
                command: "git branch".to_owned(),
                code: None,
                stderr: format!("`{branch}` cannot be used as a branch name"),
            });
        }
        if !self
            .branches(repository)?
            .iter()
            .any(|candidate| candidate.name == start_point)
        {
            return Err(GitError::Failed {
                command: "git branch".to_owned(),
                code: None,
                stderr: format!("`{start_point}` is not a local branch"),
            });
        }
        // Created and then switched to, rather than `checkout -b`, so the name
        // is validated by the command that only makes a ref: an unusable name
        // or one already taken fails before anything about `HEAD` has moved.
        self.run(
            repository.workdir(),
            &[
                OsStr::new("branch"),
                OsStr::new("--"),
                OsStr::new(branch),
                OsStr::new(start_point),
            ],
        )?;
        self.checkout_branch(repository, branch)
    }

    fn delete_branch(&self, repository: &Repository, branch: &str, force: bool) -> Result<()> {
        let branches = self.branches(repository)?;
        let Some(target) = branches.iter().find(|candidate| candidate.name == branch) else {
            return Err(GitError::Failed {
                command: "git branch".to_owned(),
                code: None,
                stderr: format!("`{branch}` is not a local branch"),
            });
        };
        if target.current {
            return Err(GitError::Failed {
                command: "git branch".to_owned(),
                code: None,
                stderr: format!("`{branch}` is the branch this working tree is on"),
            });
        }
        self.run(
            repository.workdir(),
            &[
                OsStr::new("branch"),
                OsStr::new(if force { "-D" } else { "-d" }),
                OsStr::new("--"),
                OsStr::new(branch),
            ],
        )
        .map(|_| ())
    }

    fn prepare_branch_deletion(
        &self,
        repository: &Repository,
        branch: &str,
    ) -> Result<BranchDeletionPlan> {
        self.review_branch_deletion(repository, branch, None)
    }

    fn prepare_branch_deletion_through(
        &self,
        repository: &Repository,
        branch: &str,
        checkout: &Path,
    ) -> Result<BranchDeletionPlan> {
        self.review_branch_deletion(repository, branch, Some(checkout))
    }

    fn delete_branch_guarded(
        &self,
        repository: &Repository,
        plan: &BranchDeletionPlan,
        authorization: DeletionAuthorization,
    ) -> Result<()> {
        let current = self.prepare_branch_deletion(repository, &plan.branch)?;
        if current != *plan {
            return Err(GitError::Failed {
                command: "git branch".to_owned(),
                code: None,
                stderr: "the branch changed after it was reviewed; review the deletion again"
                    .to_owned(),
            });
        }
        if authorization < current.required_authorization {
            return Err(GitError::Failed {
                command: "git branch".to_owned(),
                code: None,
                stderr: "this branch has unpublished history and needs typed confirmation"
                    .to_owned(),
            });
        }
        self.run(
            repository.workdir(),
            &[
                OsStr::new("branch"),
                OsStr::new("-D"),
                OsStr::new("--"),
                OsStr::new(&plan.branch),
            ],
        )
        .map(|_| ())
    }

    fn staged_content(&self, repository: &Repository, path: &Path) -> Result<BaseContent> {
        let relative = self.relative(repository, path)?;

        // The index entry is read before the blob so an untracked path is a
        // value rather than a failure to be recognised by its message, and so
        // a path mid-merge — which has entries at stages one to three and none
        // at zero — is reported as having no base at all.
        let entries = self.run(
            repository.workdir(),
            &[
                OsStr::new("--literal-pathspecs"),
                OsStr::new("ls-files"),
                OsStr::new("--stage"),
                OsStr::new("-z"),
                OsStr::new("--"),
                relative.as_os_str(),
            ],
        )?;
        let Some(object) = staged_object(&entries) else {
            return Ok(BaseContent::Absent);
        };

        let content = self.run(
            repository.workdir(),
            &[
                OsStr::new("cat-file"),
                OsStr::new("blob"),
                OsStr::new(&object),
            ],
        )?;
        if crate::external_open::is_binary(&content, true) {
            return Ok(BaseContent::Binary);
        }
        String::from_utf8(content)
            .map(BaseContent::Text)
            .or(Ok(BaseContent::Binary))
    }

    fn file_comparison(
        &self,
        repository: &Repository,
        scope: DiffScope,
        path: &Path,
    ) -> Result<FileComparison> {
        let current = self.staged_content(repository, path)?;
        match scope {
            DiffScope::Staged => Ok(FileComparison {
                previous: self.head_content(repository, path)?,
                current,
            }),
            DiffScope::Unstaged => Ok(FileComparison {
                previous: current,
                current: self.working_content(repository, path)?,
            }),
        }
    }

    fn diff(
        &self,
        repository: &Repository,
        scope: DiffScope,
        path: Option<&Path>,
    ) -> Result<String> {
        // External diff drivers and textconv filters are configured per
        // repository, so leaving them on would let a checkout decide what
        // program runs when someone opens a diff.
        let mut arguments = vec![
            OsString::from("--literal-pathspecs"),
            OsString::from("diff"),
            OsString::from("--no-ext-diff"),
            OsString::from("--no-textconv"),
            OsString::from("--no-color"),
        ];
        if scope == DiffScope::Staged {
            arguments.push(OsString::from("--cached"));
        }
        if let Some(path) = path {
            let relative = self.relative(repository, path)?;
            arguments.push(OsString::from("--"));
            arguments.push(relative.as_os_str().to_owned());
        }
        self.run_raw_text(repository.workdir(), &arguments)
    }

    fn stage(&self, repository: &Repository, path: &Path) -> Result<()> {
        let relative = self.relative(repository, path)?;
        self.run(
            repository.workdir(),
            &[
                OsStr::new("--literal-pathspecs"),
                OsStr::new("add"),
                OsStr::new("--"),
                relative.as_os_str(),
            ],
        )
        .map(|_| ())
    }

    fn discard(&self, repository: &Repository, path: &Path) -> Result<()> {
        let relative = self.relative(repository, path)?;
        let entries = self.run(
            repository.workdir(),
            &[
                OsStr::new("--literal-pathspecs"),
                OsStr::new("ls-files"),
                OsStr::new("--stage"),
                OsStr::new("-z"),
                OsStr::new("--"),
                relative.as_os_str(),
            ],
        )?;
        if staged_mode(&entries) == Some("160000") {
            return Err(GitError::Failed {
                command: format!("discard {}", relative.display()),
                code: None,
                stderr: "the path is a submodule; refusing to remove files below it".to_owned(),
            });
        }
        match std::fs::symlink_metadata(path) {
            Ok(metadata) if metadata.file_type().is_dir() => {
                return Err(GitError::Failed {
                    command: format!("discard {}", relative.display()),
                    code: None,
                    stderr: "the path is a directory; refusing to remove files below it".to_owned(),
                });
            }
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(GitError::Io {
                    action: "inspect",
                    path: path.to_path_buf(),
                    detail: error.to_string(),
                });
            }
        }
        let comparison = self.file_comparison(repository, DiffScope::Staged, path)?;
        if comparison.previous != BaseContent::Absent {
            // `checkout HEAD --` predates `git restore` and resets both the
            // index and worktree for a path which HEAD owns.
            return self
                .run(
                    repository.workdir(),
                    &[
                        OsStr::new("--literal-pathspecs"),
                        OsStr::new("checkout"),
                        OsStr::new("HEAD"),
                        OsStr::new("--"),
                        relative.as_os_str(),
                    ],
                )
                .map(|_| ());
        }
        if comparison.current == BaseContent::Absent {
            // A path absent from both HEAD and the index is merely untracked.
            // Discard never crosses into `git clean` for one the caller did
            // not first stage and explicitly confirm.
            return Err(GitError::Failed {
                command: format!("discard {}", relative.display()),
                code: None,
                stderr: "the path is untracked and has no committed version to restore".to_owned(),
            });
        }

        // A staged addition (including the destination half of a rename) has
        // no HEAD path for checkout to restore. `git rm` removes that one
        // indexed endpoint and its worktree file as a single operation. It
        // also refuses when the file has been replaced by a directory, so
        // discard cannot cross into unrelated untracked children. This keeps
        // the provider's pre-2.23 Git compatibility without using `clean`.
        self.run(
            repository.workdir(),
            &[
                OsStr::new("--literal-pathspecs"),
                OsStr::new("rm"),
                OsStr::new("-f"),
                OsStr::new("--"),
                relative.as_os_str(),
            ],
        )
        .map(|_| ())
    }

    fn pull(&self, repository: &Repository) -> Result<String> {
        self.upstream_branch(repository, "git pull", "pull into", "pull from")?;
        // The fetch and the merge are run separately rather than as one `git
        // pull`, because the drift below is read from the remote-tracking refs
        // and those are only worth reading once a fetch has actually refreshed
        // them. A single `pull` that fails says nothing about whether its fetch
        // got that far: an unreachable remote and a branch that cannot
        // fast-forward both come back as one failure, and the refs still hold
        // whatever the last successful fetch left. Reporting divergence from
        // those would turn "the remote did not answer" into an offer to rebase
        // onto a tip nobody has seen.
        self.run_network(repository.workdir(), &["fetch"])?;
        if let Some(diverged) = self.divergence(repository) {
            return Err(diverged);
        }
        // `--no-autostash` because a pull that stashes uncommitted changes,
        // fast-forwards, and then fails to reapply them exits successfully
        // while leaving conflict markers in the working tree and a stash to
        // recover — which is the state this whole path exists to avoid, and
        // one a caller reading the exit status cannot see. Refusing a dirty
        // tree up front is an outcome the reader can act on. `merge` rather
        // than `pull --ff-only` follows from the split above; `--ff-only`
        // still rules out both a merge commit and a rebase.
        self.run_network(
            repository.workdir(),
            &[
                "merge",
                "--ff-only",
                "--no-autostash",
                "--no-stat",
                "@{upstream}",
            ],
        )
    }

    fn rebase_onto_upstream(&self, repository: &Repository) -> Result<String> {
        let branch = self.upstream_branch(repository, "git rebase", "rebase", "rebase onto")?;
        // `git pull --rebase` rather than `git rebase @{upstream}`: the remote
        // may have moved again since the pull that reported the divergence,
        // and refetching means the commits being replayed land on what the
        // remote holds now rather than on a tip that is already stale.
        // `--no-autostash` for the reason `pull` above passes it, which bites
        // harder here: with `rebase.autoStash` set, a replay over uncommitted
        // changes whose reapplication conflicts exits *successfully*, having
        // rebased, leaving conflict markers in the working tree and the stash
        // behind. Nothing is mid-rebase afterwards, so the rollback below never
        // runs and the caller reports success over a conflicted tree. Refusing
        // a dirty worktree up front is the only answer that keeps the promise
        // this function makes.
        match self.run_network(
            repository.workdir(),
            &["pull", "--rebase", "--no-autostash", "--no-stat"],
        ) {
            Ok(summary) => Ok(summary),
            Err(error) => {
                // A rebase that stops mid-replay leaves the working tree
                // holding conflict markers and `HEAD` detached partway up the
                // branch, which is precisely the state Runyte has no surface
                // to finish. Undoing it puts the reader back where they
                // started, with the refusal as the only thing that changed.
                //
                // The undo runs uncancelled. `:git-cancel` stops the replay by
                // setting a flag this provider checks before every wait, so
                // cleanup sharing it would find the probe refused and the abort
                // killed — leaving the half-finished tree that cancelling was
                // supposed to spare the reader. Cancellation is not rollback
                // everywhere else because nothing else here knows how to roll
                // back; a stopped rebase does.
                let cleanup = self.uncancellable();
                if !cleanup.rebase_in_progress(repository) {
                    return Err(error);
                }
                if let Err(abort) = cleanup.run(repository.workdir(), &["rebase", "--abort"]) {
                    return Err(GitError::Failed {
                        command: "git rebase --abort".to_owned(),
                        code: None,
                        stderr: format!(
                            "replaying {branch} stopped partway and undoing it failed ({abort}); \
                             the working tree is mid-rebase. Finish or abort it with `git rebase` \
                             outside Runyte"
                        ),
                    });
                }
                // A cancelled replay keeps its own error, so the service's
                // cancellation bookkeeping still recognises it; what changed is
                // that the tree it reconciles is the one the reader started
                // with.
                if matches!(error, GitError::Cancelled { .. }) {
                    return Err(error);
                }
                Err(GitError::Failed {
                    command: "git pull --rebase".to_owned(),
                    code: None,
                    // A status line is one row wide, so the part that says the
                    // repository is untouched comes before the part that says
                    // where to go next.
                    stderr: format!(
                        "replaying {branch} hit a conflict; the rebase was undone. Finish it with \
                         `git rebase` outside Runyte"
                    ),
                })
            }
        }
    }

    fn push(&self, repository: &Repository, branch: &str) -> Result<String> {
        let branches = self.branches(repository)?;
        let Some(target) = branches.iter().find(|candidate| candidate.name == branch) else {
            return Err(GitError::Failed {
                command: "git push".to_owned(),
                code: None,
                stderr: format!("`{branch}` is not a local branch"),
            });
        };
        // The refspec is written out in full in both cases. Leaving it to
        // `push.default` would make what this key does depend on a
        // configuration Runyte never shows, and the two cases genuinely differ:
        // one publishes to a ref that exists, the other creates it.
        let (remote, refspec, set_upstream) = match &target.upstream {
            Some(upstream) if upstream.divergence.is_some() => {
                validate_push_destination(&upstream.remote, Some(&upstream.reference))?;
                (
                    upstream.remote.clone(),
                    format!("{branch}:{}", upstream.reference),
                    false,
                )
            }
            // An upstream that is configured and gone is republished the same
            // way an absent one is: the ref has to be created again, and the
            // configuration already names where.
            Some(upstream) => {
                validate_push_destination(&upstream.remote, Some(&upstream.reference))?;
                (
                    upstream.remote.clone(),
                    format!("{branch}:{}", upstream.reference),
                    true,
                )
            }
            None => {
                let remote = self.default_remote(repository)?;
                validate_push_destination(&remote, None)?;
                (remote, branch.to_owned(), true)
            }
        };
        let mut arguments = vec![OsString::from("push")];
        if set_upstream {
            arguments.push(OsString::from("--set-upstream"));
        }
        arguments.extend([
            OsString::from("--"),
            OsString::from(remote),
            OsString::from(refspec),
        ]);
        match self.run_network(repository.workdir(), &arguments) {
            Ok(summary) => Ok(summary),
            // Git says what to do about a rejected push in a `hint:` line, and
            // `without_noise` drops those along with the rest. Saying it here
            // keeps the one refusal that has a next step from reading as a
            // dead end — which, with a pull that refuses a diverged branch on
            // the other side, is exactly how it read.
            Err(GitError::Failed {
                command,
                code,
                stderr,
            }) if rejected_as_stale(&stderr) => Err(GitError::Failed {
                command,
                code,
                stderr: format!(
                    "{stderr}; {branch} is behind what the remote holds. Pull it first, which \
                     offers to replay these commits on top"
                ),
            }),
            Err(error) => Err(error),
        }
    }

    fn commit(&self, repository: &Repository, message: &str) -> Result<String> {
        // The message is one argument vector element, so nothing in it can be
        // read as an option or as syntax however it is written.
        //
        // `--cleanup=whitespace` because the comment lines have already been
        // removed here, the way Git removes them after an editor session; a
        // stricter mode would then go on to strip content nobody asked it to.
        self.run_text(
            repository.workdir(),
            &[
                OsStr::new("commit"),
                OsStr::new("--cleanup=whitespace"),
                OsStr::new("-m"),
                OsStr::new(message),
            ],
        )
    }

    fn unstage(&self, repository: &Repository, path: &Path) -> Result<()> {
        // `reset` rather than `restore --staged`: it means the same thing for
        // one path, it predates `restore` by years, and it is the only one of
        // the two that works before the first commit, where `HEAD` resolves to
        // nothing and unstaging means dropping the entry entirely.
        let relative = self.relative(repository, path)?;
        self.run(
            repository.workdir(),
            &[
                OsStr::new("--literal-pathspecs"),
                OsStr::new("reset"),
                OsStr::new("-q"),
                OsStr::new("--"),
                relative.as_os_str(),
            ],
        )
        .map(|_| ())
    }
}

/// Refuses repository-controlled values before they become push arguments.
///
/// A remote name is a positional argument only after `--`, which `push` also
/// supplies as defense in depth. Keeping this validation at the consuming
/// boundary means a malformed upstream remains visible in the branch list but
/// can never become an option if the command's argument order changes later.
fn validate_push_destination(remote: &str, reference: Option<&str>) -> Result<()> {
    let invalid = if remote.starts_with('-') {
        Some("remote names")
    } else if reference.is_some_and(|reference| reference.starts_with('-')) {
        Some("remote refs")
    } else {
        None
    };
    if let Some(kind) = invalid {
        return Err(GitError::Failed {
            command: "git push".to_owned(),
            code: None,
            stderr: format!("{kind} beginning with `-` are refused"),
        });
    }
    Ok(())
}

/// Whether a push was refused because the remote holds commits this branch
/// does not, rather than for any of the other reasons a push can fail.
///
/// Both wordings are matched: Git says `non-fast-forward` when the local tip
/// is simply behind, and `fetch first` when the remote ref moved to something
/// unrelated to what was last fetched.
fn rejected_as_stale(stderr: &str) -> bool {
    stderr.contains("non-fast-forward") || stderr.contains("fetch first")
}

/// What a terminal would have been left showing, for output Git wrote a line
/// of progress at a time.
///
/// Counters like `Rebasing (1/2)` and `Receiving objects: 40%` are redrawn in
/// place with a carriage return rather than appended, so reading the bytes as
/// text runs every intermediate step together with the sentence that follows
/// it. Keeping only what comes after the last carriage return on each line is
/// what the terminal itself does, and it leaves the outcome rather than the
/// journey to it.
fn settled(output: &str) -> String {
    output
        .lines()
        .map(|line| line.rsplit('\r').next().unwrap_or(line))
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_owned()
}

/// Git's diagnostics, reduced to the part that says what happened.
///
/// A status line is one line, and Git does not write its failures with that in
/// mind. A refused merge opens with several `hint:` lines of multi-line advice
/// and puts the `fatal:` last; a refused push opens with the `To <destination>`
/// header it writes on every push, success or not. Both push the sentence that
/// matters past the right-hand edge. The hints cannot fit anywhere anyway, and
/// the destination is already named by the command this message quotes, so
/// dropping them lets the rest lead.
///
/// If a message were nothing but those, keeping them beats saying nothing.
/// What is dropped along with them has to be said elsewhere: a rejected push
/// says how to catch up itself, because the `hint:` line that said so is one
/// of the casualties here.
fn without_noise(stderr: &str) -> String {
    let stderr = settled(stderr);
    let kept = stderr
        .lines()
        .map(str::trim)
        .filter(|line| {
            !line.trim_start().starts_with("hint:") && !line.starts_with("To ") && !line.is_empty()
        })
        .collect::<Vec<_>>()
        .join("; ");
    if kept.is_empty() {
        stderr.trim().to_owned()
    } else {
        kept
    }
}

/// What `%(upstream:short)` and `%(upstream:track)` say about one branch.
///
/// Git writes the tracking field as `[ahead 2, behind 1]`, as `[gone]` when the
/// upstream ref has been removed, and as nothing at all when the two are in
/// step. An empty upstream name means none is configured, which is not the same
/// as one that is configured and missing.
fn parse_upstream(name: &str, remote: &str, reference: &str, track: &str) -> Option<Upstream> {
    if name.is_empty() {
        return None;
    }
    let track = track.trim();
    if track == "[gone]" {
        return Some(Upstream {
            name: name.to_owned(),
            remote: remote.to_owned(),
            reference: reference.to_owned(),
            divergence: None,
        });
    }
    let mut divergence = Divergence::default();
    for part in track
        .trim_start_matches('[')
        .trim_end_matches(']')
        .split(',')
    {
        let mut words = part.split_whitespace();
        let (Some(direction), Some(count)) = (words.next(), words.next()) else {
            continue;
        };
        let Ok(count) = count.parse::<usize>() else {
            continue;
        };
        match direction {
            "ahead" => divergence.ahead = count,
            "behind" => divergence.behind = count,
            _ => {}
        }
    }
    Some(Upstream {
        name: name.to_owned(),
        remote: remote.to_owned(),
        reference: reference.to_owned(),
        divergence: Some(divergence),
    })
}

/// The object id of the stage-zero entry in `git ls-files --stage -z` output.
///
/// Each record is `<mode> <object> <stage>\t<path>`. A path in conflict has no
/// stage-zero entry, and so has no staged content to compare against.
fn staged_object(entries: &[u8]) -> Option<String> {
    entries
        .split(|byte| *byte == 0)
        .filter(|entry| !entry.is_empty())
        .find_map(|entry| {
            let head = entry.split(|byte| *byte == b'\t').next()?;
            let mut fields = std::str::from_utf8(head).ok()?.split_whitespace();
            let _mode = fields.next()?;
            let object = fields.next()?;
            (fields.next()? == "0").then(|| object.to_owned())
        })
}

fn staged_mode(entries: &[u8]) -> Option<&str> {
    entries
        .split(|byte| *byte == 0)
        .filter(|entry| !entry.is_empty())
        .find_map(|entry| {
            let head = entry.split(|byte| *byte == b'\t').next()?;
            let mut fields = std::str::from_utf8(head).ok()?.split_whitespace();
            let mode = fields.next()?;
            let _object = fields.next()?;
            (fields.next()? == "0").then_some(mode)
        })
}

/// The object id in one `git ls-tree -z` record.
fn tree_object(entries: &[u8]) -> Option<String> {
    let head = entries
        .split(|byte| *byte == b'\t')
        .next()
        .filter(|head| !head.is_empty())?;
    let mut fields = std::str::from_utf8(head).ok()?.split_whitespace();
    let _mode = fields.next()?;
    (fields.next()? == "blob").then(|| fields.next().map(str::to_owned))?
}

fn read_file_for_comparison(path: &Path, limit: usize) -> Result<Vec<u8>> {
    let file = std::fs::File::open(path).map_err(|error| GitError::Io {
        action: "read",
        path: path.to_path_buf(),
        detail: error.to_string(),
    })?;
    let mut content = Vec::new();
    file.take(limit as u64 + 1)
        .read_to_end(&mut content)
        .map_err(|error| GitError::Io {
            action: "read",
            path: path.to_path_buf(),
            detail: error.to_string(),
        })?;
    if content.len() > limit {
        return Err(GitError::TooLarge {
            command: format!("read {}", path.display()),
            limit,
        });
    }
    Ok(content)
}

fn bounded_file_sha256(repository: &Repository, path: &Path) -> Result<String> {
    let is_within = path
        .strip_prefix(repository.workdir())
        .ok()
        .is_some_and(|relative| {
            !relative.components().any(|component| {
                matches!(
                    component,
                    Component::ParentDir | Component::RootDir | Component::Prefix(_)
                )
            })
        });
    if !is_within {
        return Err(GitError::NotARepository {
            path: path.to_path_buf(),
        });
    }
    let metadata = std::fs::symlink_metadata(path).map_err(|error| GitError::Io {
        action: "fingerprint",
        path: path.to_path_buf(),
        detail: error.to_string(),
    })?;
    if !metadata.file_type().is_file()
        || crate::path_safety::ensure_within_root(repository.workdir(), path).is_err()
    {
        return Err(GitError::NotARepository {
            path: path.to_path_buf(),
        });
    }
    let file = std::fs::File::open(path).map_err(|error| GitError::Io {
        action: "fingerprint",
        path: path.to_path_buf(),
        detail: error.to_string(),
    })?;
    let mut bytes = Vec::new();
    file.take(MAX_PATCH_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| GitError::Io {
            action: "fingerprint",
            path: path.to_path_buf(),
            detail: error.to_string(),
        })?;
    if bytes.len() > MAX_PATCH_BYTES {
        return Err(GitError::TooLarge {
            command: "partial-stage file fingerprint".to_owned(),
            limit: MAX_PATCH_BYTES,
        });
    }
    Ok(crate::hash::sha256_hex(&bytes))
}

fn stash_apply_error(error: GitError) -> GitError {
    match error {
        GitError::Failed {
            command,
            code,
            stderr,
        } => GitError::Failed {
            command,
            code,
            stderr: format!(
                "{stderr}{}the stash was retained; resolve conflicts with an external Git tool",
                if stderr.is_empty() { "" } else { "; " }
            ),
        },
        error => error,
    }
}

fn stale_partial<T>() -> Result<T> {
    Err(GitError::Failed {
        command: "partial staging".to_owned(),
        code: None,
        stderr: "stale patch: HEAD, index, or working-tree file changed; refresh and retry"
            .to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::BufferRevisionGuard;

    #[test]
    fn long_failure_logs_are_large_and_explicitly_bounded() {
        let short = vec![b'x'; 128 * 1024];
        let finish = PipeFinalizer::new().unwrap();
        assert_eq!(
            read_bounded_stderr(short.as_slice(), &finish.signal()),
            (short, true)
        );

        let long = vec![b'x'; MAX_STDERR_BYTES + 100];
        let mut source = std::io::Cursor::new(long.as_slice());
        let (retained, eof) = read_bounded_stderr(&mut source, &finish.signal());
        assert!(retained.starts_with(&long[..MAX_STDERR_BYTES]));
        assert!(retained.ends_with(STDERR_TRUNCATED));
        assert!(eof);
        assert_eq!(source.position(), long.len() as u64, "the pipe was drained");
    }

    #[test]
    fn oversized_stdout_is_signalled_after_the_retained_bound() {
        let source = vec![b'x'; 4096];
        let exceeded = AtomicBool::new(false);
        let finish = PipeFinalizer::new().unwrap();
        let (retained, read, eof) =
            read_bounded_output(source.as_slice(), 128, &exceeded, &finish.signal());

        read.unwrap();
        assert!(eof);
        assert_eq!(retained.len(), 129);
        assert!(exceeded.load(Ordering::Acquire));
    }

    #[test]
    fn inherited_git_authority_is_removed_from_every_command() {
        let provider = GitCliProvider::new("git");
        let command = provider.command(Path::new("."), &["status"], false, false);
        assert_eq!(
            command.get_args().collect::<Vec<_>>(),
            [OsStr::new("--no-optional-locks"), OsStr::new("status")]
        );
        let environment = command
            .get_envs()
            .map(|(name, value)| (name.to_owned(), value.map(OsStr::to_owned)))
            .collect::<std::collections::HashMap<_, _>>();

        for name in [
            "GIT_DIR",
            "GIT_WORK_TREE",
            "GIT_INDEX_FILE",
            "GIT_OBJECT_DIRECTORY",
            "GIT_ALTERNATE_OBJECT_DIRECTORIES",
            "GIT_COMMON_DIR",
            "GIT_NAMESPACE",
            "GIT_REPLACE_REF_BASE",
            "GIT_SHALLOW_FILE",
            "GIT_GRAFT_FILE",
            "GIT_CONFIG_PARAMETERS",
            "GIT_CONFIG_COUNT",
            "GIT_EXEC_PATH",
            "GIT_TEMPLATE_DIR",
        ] {
            assert_eq!(environment.get(OsStr::new(name)), Some(&None), "{name}");
        }
        assert_eq!(
            environment.get(OsStr::new("GIT_TERMINAL_PROMPT")),
            Some(&Some(OsString::from("0")))
        );

        let mut command = Command::new("git");
        remove_inherited_config_entries(
            &mut command,
            [
                OsString::from("GIT_CONFIG_KEY_0"),
                OsString::from("GIT_CONFIG_VALUE_0"),
                OsString::from("GIT_CONFIG_OTHER"),
            ],
        );
        let removed = command
            .get_envs()
            .map(|(name, value)| (name.to_owned(), value.map(OsStr::to_owned)))
            .collect::<std::collections::HashMap<_, _>>();
        assert_eq!(removed.get(OsStr::new("GIT_CONFIG_KEY_0")), Some(&None));
        assert_eq!(removed.get(OsStr::new("GIT_CONFIG_VALUE_0")), Some(&None));
        assert!(!removed.contains_key(OsStr::new("GIT_CONFIG_OTHER")));
    }

    #[cfg(unix)]
    #[test]
    fn local_reads_refuse_traversal_and_symlinked_parent_escapes() {
        use std::os::unix::fs::symlink;

        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let base = std::env::temp_dir().join(format!(
            "runyte-git-local-path-{}-{nonce}",
            std::process::id()
        ));
        let root = base.join("repository");
        let outside = base.join("outside");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("secret.txt"), "outside\n").unwrap();
        symlink(&outside, root.join("escape")).unwrap();
        let repository = Repository::new(&root);
        let provider = GitCliProvider::new("git");

        assert!(matches!(
            provider.relative(&repository, &root.join("../outside/secret.txt")),
            Err(GitError::NotARepository { .. })
        ));
        assert!(matches!(
            provider.working_content(&repository, &root.join("escape/secret.txt")),
            Err(GitError::NotARepository { .. })
        ));
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn failure_output_retains_and_labels_both_bounded_streams() {
        let output = failure_output(b"hook context\n", b"fatal: refused\n");
        assert_eq!(output, "stdout:\nhook context\nstderr:\nfatal: refused");

        let large = "界".repeat(MAX_FAILURE_OUTPUT_BYTES);
        let bounded = failure_output_text(&large, &large);
        assert!(bounded.len() <= MAX_FAILURE_OUTPUT_BYTES);
        assert!(bounded.contains("stdout:\n"));
        assert!(bounded.contains("stderr:\n"));
        assert!(bounded.contains("Runyte truncated stdout"));
        assert!(bounded.contains("Runyte truncated stderr"));
    }

    /// Cleanup after a cancellation must not itself be cancellable.
    ///
    /// `rebase_onto_upstream` promises never to leave a working tree mid-
    /// replay, and it keeps that promise by probing for a stopped rebase and
    /// aborting it. Both of those are Git commands on the same provider, so a
    /// flag that is still set — which is exactly the case when cancellation is
    /// what stopped the replay — would refuse the probe and kill the abort,
    /// leaving the state cancelling was meant to spare the reader.
    ///
    /// The stand-in waits on a file rather than on a clock, so a command that
    /// exits before the first cancellation poll — which is what a loaded
    /// machine would otherwise produce — cannot slip past it. Removing the
    /// file releases the same program for the run that has to reach Git.
    #[cfg(unix)]
    #[test]
    fn cleanup_after_a_cancellation_still_runs_git() {
        use std::os::unix::fs::PermissionsExt;

        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root =
            std::env::temp_dir().join(format!("runyte-git-cleanup-{}-{nonce}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let program = root.join("slow-git");
        std::fs::write(
            &program,
            "#!/bin/sh\nwhile [ -e \"$0.hold\" ]; do sleep 0.05; done\necho ran\n",
        )
        .unwrap();
        let hold = root.join("slow-git.hold");
        std::fs::write(&hold, b"").unwrap();
        let mut permissions = std::fs::metadata(&program).unwrap().permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&program, permissions).unwrap();
        let cancellation = Arc::new(AtomicBool::new(true));
        let provider = GitCliProvider::new(&program).with_cancellation(Arc::clone(&cancellation));

        let error = provider.run(&root, &["rebase", "--abort"]).unwrap_err();
        assert!(matches!(error, GitError::Cancelled { .. }), "{error:?}");

        // The same provider, with the flag dropped, still reaches Git — which
        // is what lets the rollback undo a replay the flag just stopped.
        std::fs::remove_file(&hold).unwrap();
        let output = provider
            .uncancellable()
            .run_text(&root, &["rebase", "--abort"])
            .expect("cleanup must still be able to run Git");
        assert_eq!(output, "ran");

        // Dropping the flag is scoped to the clone: the original still observes
        // it, so nothing else on this provider becomes uncancellable.
        std::fs::write(&hold, b"").unwrap();
        assert!(matches!(
            provider.run(&root, &["rebase", "--abort"]).unwrap_err(),
            GitError::Cancelled { .. }
        ));
        std::fs::remove_dir_all(root).unwrap();
    }

    /// Progress counters are redrawn over, not appended to, so a summary that
    /// reads the bytes as text has to resolve the redrawing itself.
    #[test]
    fn progress_counters_settle_to_what_the_terminal_would_show() {
        assert_eq!(
            settled("Rebasing (1/2)\rRebasing (2/2)\rSuccessfully rebased and updated main.\n"),
            "Successfully rebased and updated main."
        );
        // Each line settles on its own, so a counter cannot swallow the line
        // above it.
        assert_eq!(
            settled("From /somewhere\n   abc..def  main -> origin/main\nFast-forward\n"),
            "From /somewhere\n   abc..def  main -> origin/main\nFast-forward"
        );
        assert_eq!(
            settled("Receiving objects: 40%\rReceiving objects: 100%\r"),
            ""
        );
    }

    #[test]
    fn invalid_live_buffer_guard_refuses_before_any_repository_read() {
        let guard = BufferRevisionGuard::new();
        guard.invalidate();
        let request = PartialStageRequest {
            repository: PathBuf::from("/never-read/.git"),
            fingerprint: RepositoryFingerprint {
                head: None,
                index: "0".repeat(64),
            },
            path: PathBuf::from("/never-read/source.txt"),
            disk_sha256: "0".repeat(64),
            buffer: Some((
                crate::workspace::BufferId::from_index(0),
                crate::workspace::BufferRevision::from_raw(1),
            )),
            guard: Some(guard),
            scope: DiffScope::Unstaged,
            hunk: "0".repeat(64),
            patch: b"not reached".to_vec(),
        };
        let error = GitCliProvider::new("git")
            .apply_partial(&Repository::new("/never-read"), &request)
            .unwrap_err();
        assert!(error.to_string().contains("stale patch"), "{error}");
    }

    #[cfg(unix)]
    #[test]
    fn local_history_reads_have_a_deadline() {
        use std::{fs, os::unix::fs::PermissionsExt, time::Duration};

        let directory = std::env::temp_dir().join(format!(
            "runyte-git-read-timeout-{}-{}",
            std::process::id(),
            crate::hash::sha256_hex(format!("{:?}", std::time::Instant::now()).as_bytes())
        ));
        fs::create_dir_all(&directory).unwrap();
        let program = directory.join("slow-git");
        fs::write(&program, "#!/bin/sh\nsleep 1\n").unwrap();
        fs::set_permissions(&program, fs::Permissions::from_mode(0o700)).unwrap();
        let provider =
            GitCliProvider::new(&program).with_local_read_timeout(Duration::from_millis(10));
        let error = provider
            .log_page(&Repository::new(&directory), &LogRequest::default())
            .unwrap_err();
        assert!(matches!(error, GitError::TimedOut { .. }), "{error:?}");
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn the_staged_object_is_the_stage_zero_entry() {
        let entries = b"100644 1a2b3c4d 0\tsrc/main.rs\0".as_slice();
        assert_eq!(staged_object(entries).as_deref(), Some("1a2b3c4d"));
    }

    /// A path in the middle of a merge has stages one to three and no zero,
    /// so there is no single text it used to be.
    #[test]
    fn a_conflicted_path_has_no_staged_object() {
        let entries =
            b"100644 aaaa 1\tclash.rs\x00100644 bbbb 2\tclash.rs\x00100644 cccc 3\tclash.rs\x00"
                .as_slice();
        assert_eq!(staged_object(entries), None);
        assert_eq!(staged_object(b"".as_slice()), None);
    }

    /// A commit message is an argument, and it can be as long as anyone
    /// likes; naming the command back must not reprint it whole.
    #[test]
    fn a_named_command_stays_short_enough_to_read() {
        let provider = GitCliProvider::new("git");
        let message = "A subject line, and then a body that goes on at some \
             considerable length about what changed and why";

        let described = provider.describe(&["commit", "-m", message]);

        assert!(
            described.starts_with("git commit -m A subject line"),
            "{described}"
        );
        assert!(described.ends_with('…'), "{described}");
        assert_eq!(described.chars().count(), MAX_DESCRIPTION_CHARS + 1);
        // A short command is named in full.
        assert_eq!(
            provider.describe(&["rev-parse", "HEAD"]),
            "git rev-parse HEAD"
        );
    }

    /// The four things Git's tracking field can say, and the one thing an
    /// empty upstream name means whatever the field holds.
    #[test]
    fn tracking_is_read_from_the_field_git_writes() {
        let parsed = |track| parse_upstream("origin/main", "origin", "refs/heads/main", track);
        assert_eq!(parse_upstream("", "", "", ""), None);
        assert_eq!(
            parse_upstream("", "origin", "refs/heads/x", "[ahead 1]"),
            None
        );
        assert_eq!(
            parsed(""),
            Some(Upstream::origin("main", Some(Divergence::default())))
        );
        // The remote and its ref survive an upstream that no longer exists,
        // because pushing the branch again is what recreates it.
        assert_eq!(
            parsed("[gone]"),
            Some(Upstream {
                name: "origin/main".to_owned(),
                remote: "origin".to_owned(),
                reference: "refs/heads/main".to_owned(),
                divergence: None,
            })
        );
        assert_eq!(
            parsed("[ahead 2, behind 1]").and_then(|upstream| upstream.divergence),
            Some(Divergence {
                ahead: 2,
                behind: 1,
            })
        );
        assert_eq!(
            parsed("[behind 3]").and_then(|upstream| upstream.divergence),
            Some(Divergence {
                ahead: 0,
                behind: 3,
            })
        );
        // A remote whose name contains the separator the short form is built
        // with is still read correctly, because neither field is derived from
        // the other.
        let odd = parse_upstream("fork/team/main", "fork/team", "refs/heads/main", "").unwrap();
        assert_eq!(odd.remote, "fork/team");
        assert_eq!(odd.reference, "refs/heads/main");
    }

    #[test]
    fn option_shaped_remote_refs_are_not_push_destinations() {
        let error = validate_push_destination("origin", Some("--upload-pack=helper")).unwrap_err();

        assert!(
            error
                .to_string()
                .contains("remote refs beginning with `-` are refused"),
            "{error}"
        );
    }

    /// What a refused fast-forward actually looks like: the sentence that says
    /// what happened is last, behind advice that cannot fit on a status line.
    #[test]
    fn a_network_failure_leads_with_what_happened_rather_than_advice() {
        let stderr = "hint: Diverging branches can't be fast-forwarded, you need to either:\n\
                      hint:\n\
                      hint: \tgit merge --no-ff\n\
                      fatal: Not possible to fast-forward, aborting.\n";

        assert_eq!(
            without_noise(stderr),
            "fatal: Not possible to fast-forward, aborting."
        );
        // A rejected push loses the destination header it opens with — the
        // command quoted alongside this already names the remote — and keeps
        // its remaining lines in the order Git wrote them.
        assert_eq!(
            without_noise(
                "To /srv/repositories/project.git\n\
                 ! [rejected]        main -> main (fetch first)\n\
                 error: failed to push some refs\n\
                 hint: Updates were rejected because the remote contains work\n"
            ),
            "! [rejected]        main -> main (fetch first); error: failed to push some refs"
        );
        // Advice is better than silence when there is nothing else.
        assert_eq!(without_noise("hint: try again\n"), "hint: try again");
        assert_eq!(without_noise(""), "");
    }

    /// Killing only the top-level Git process is not enough: a transport or
    /// credential helper can inherit its output pipes and keep the reader
    /// threads blocked after the advertised deadline.
    #[cfg(unix)]
    #[test]
    fn a_network_timeout_is_not_extended_by_a_child_holding_the_pipes() {
        use std::os::unix::fs::PermissionsExt;

        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "runyte-git-network-timeout-{}-{nonce}",
            std::process::id()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let program = root.join("git-with-helper");
        std::fs::write(&program, "#!/bin/sh\nsleep 30 &\nwait\n").unwrap();
        let mut permissions = std::fs::metadata(&program).unwrap().permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&program, permissions).unwrap();
        let provider = GitCliProvider::new(&program);
        let started = std::time::Instant::now();

        let error = provider
            .run_network_with_timeout(&root, &["pull"], std::time::Duration::from_millis(100))
            .unwrap_err();

        assert!(matches!(error, GitError::TimedOut { .. }), "{error:?}");
        assert!(
            started.elapsed() < std::time::Duration::from_secs(2),
            "a helper holding the pipes extended the deadline"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    /// Commit hooks and filters are ordinary Git descendants too, so service
    /// cancellation must stop their process group even for a local command.
    #[cfg(unix)]
    #[test]
    fn cancellation_stops_a_local_command_and_its_helpers() {
        use std::os::unix::fs::PermissionsExt;

        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root =
            std::env::temp_dir().join(format!("runyte-git-cancel-{}-{nonce}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let program = root.join("git-with-helper");
        std::fs::write(&program, "#!/bin/sh\nsleep 30 &\nwait\n").unwrap();
        let mut permissions = std::fs::metadata(&program).unwrap().permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&program, permissions).unwrap();
        let cancellation = Arc::new(AtomicBool::new(false));
        let provider = GitCliProvider::new(&program).with_cancellation(Arc::clone(&cancellation));
        let directory = root.clone();
        let started = std::time::Instant::now();
        let worker = std::thread::spawn(move || provider.run(&directory, &["status"]));
        std::thread::sleep(std::time::Duration::from_millis(100));
        cancellation.store(true, Ordering::Release);

        let error = worker.join().unwrap().unwrap_err();
        assert!(matches!(error, GitError::Cancelled { .. }), "{error:?}");
        assert!(
            started.elapsed() < std::time::Duration::from_secs(2),
            "a helper holding the pipes survived cancellation"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    /// A helper may detach just before Git exits successfully. Its inherited
    /// pipes must not keep the completed worker blocked in a reader join.
    #[cfg(unix)]
    #[test]
    fn a_detached_helper_cannot_hold_completed_command_pipes_open() {
        use std::os::unix::fs::PermissionsExt;

        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "runyte-git-detached-helper-{}-{nonce}",
            std::process::id()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let program = root.join("git-with-detached-helper");
        std::fs::write(&program, "#!/bin/sh\nsleep 30 &\nexit 0\n").unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
        let started = std::time::Instant::now();

        GitCliProvider::new(&program)
            .run(&root, &["status"])
            .unwrap();

        assert!(
            started.elapsed() < std::time::Duration::from_secs(2),
            "a detached helper kept the completed command's pipes open"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    /// Even a descendant that creates a new session and therefore cannot be
    /// reached through Git's process group must not turn a completed command
    /// into an unbounded reader join or change Git's completed result.
    #[cfg(unix)]
    #[test]
    fn a_session_escaping_helper_cannot_hold_the_completed_worker() {
        use std::os::unix::fs::PermissionsExt;

        if Command::new("setsid").arg("true").status().is_err() {
            return;
        }
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "runyte-git-escaped-helper-{}-{nonce}",
            std::process::id()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let program = root.join("git-with-escaped-helper");
        std::fs::write(&program, "#!/bin/sh\nsetsid sleep 2 &\nexit 0\n").unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
        let started = std::time::Instant::now();

        GitCliProvider::new(&program)
            .run(&root, &["status"])
            .unwrap();

        assert!(
            started.elapsed() < std::time::Duration::from_secs(1),
            "an escaped helper kept the completed command's pipes open"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    /// A fast child may exit before either pipe worker is scheduled. Output
    /// completion is EOF, not whether those workers happened to run within a
    /// grace period after the process status became available.
    #[cfg(unix)]
    #[test]
    fn fast_output_survives_readers_held_until_after_child_exit() {
        use std::os::unix::fs::PermissionsExt;

        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "runyte-git-gated-readers-{}-{nonce}",
            std::process::id()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let program = root.join("git-with-fast-output");
        std::fs::write(
            &program,
            "#!/bin/sh\nprintf 'complete\\n'\nprintf 'diagnostic\\n' >&2\n",
        )
        .unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
        let gate = Arc::new(TestPipeReaderGate::default());
        let provider = GitCliProvider::new(&program).with_pipe_reader_gate(gate);

        assert_eq!(provider.run(&root, &["status"]).unwrap(), b"complete\n");

        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn finalizer_wake_is_followed_by_a_fresh_eof_read() {
        use std::os::unix::fs::PermissionsExt;

        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "runyte-git-parked-readers-{}-{nonce}",
            std::process::id()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let release = root.join("release");
        let program = root.join("git-with-parked-readers");
        std::fs::write(
            &program,
            format!(
                "#!/bin/sh\nwhile [ ! -e '{}' ]; do :; done\nprintf 'complete\\n'\n",
                release.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
        let observer = Arc::new(TestPipePollObserver::default());
        let provider = GitCliProvider::new(&program).with_pipe_poll_observer(Arc::clone(&observer));
        let worker_root = root.clone();
        let worker = std::thread::spawn(move || provider.run(&worker_root, &["status"]));

        let parked = observer.wait_for(2, std::time::Duration::from_secs(2));
        std::fs::write(&release, "go\n").unwrap();
        let output = worker.join().unwrap().unwrap();

        assert!(parked, "both pipe readers did not reach kernel readiness");
        assert_eq!(output, b"complete\n");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn finalizer_wake_wins_when_pipe_data_is_ready_too() {
        use std::io::Write as _;

        let (reader, mut writer) = std::os::unix::net::UnixStream::pair().unwrap();
        let finalizer = PipeFinalizer::new().unwrap();
        let signal = finalizer.signal();
        writer.write_all(b"queued output").unwrap();
        finalizer.request_finish();

        assert!(
            !wait_for_pipe(&reader, PIPE_READ_READY, &signal).unwrap(),
            "a simultaneously readable pipe hid the finalizer wake"
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn darwin_child_exit_observer_covers_exit_before_and_after_creation() {
        use std::io::Write as _;
        use std::process::Stdio;

        let mut child = Command::new("sh")
            .arg("-c")
            .arg("read release")
            .stdin(Stdio::piped())
            .spawn()
            .unwrap();
        let observer_before_exit = ChildExitObserver::new(&child).unwrap();
        child.stdin.take().unwrap().write_all(b"go\n").unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while !observer_before_exit.exited().unwrap() {
            assert!(
                std::time::Instant::now() < deadline,
                "the observer never reached the child's stable zombie state"
            );
            std::thread::yield_now();
        }

        let observer_after_exit = ChildExitObserver::new(&child).unwrap();
        assert!(
            observer_after_exit.exited().unwrap(),
            "observer creation after exit did not recognize the unreaped zombie"
        );
        assert!(
            try_finish_child(&mut child, &observer_after_exit)
                .unwrap()
                .expect("the zombie snapshot did not finish the child")
                .success(),
            "post-zombie group cleanup changed the child status"
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn darwin_process_snapshot_requires_a_stable_zombie_before_cleanup() {
        assert_eq!(
            darwin_process_snapshot_state(libc::SRUN, DARWIN_PROC_FLAG_INEXIT),
            DarwinProcessState::Exiting,
        );
        assert_eq!(
            darwin_process_snapshot_state(libc::SZOMB, 0),
            DarwinProcessState::Zombie,
        );
        assert_eq!(
            darwin_process_snapshot_state(libc::SRUN, 0),
            DarwinProcessState::Live,
        );
    }

    #[cfg(unix)]
    #[test]
    fn failed_input_write_cannot_be_reported_as_git_success() {
        use std::os::unix::fs::PermissionsExt;

        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "runyte-git-gated-input-{}-{nonce}",
            std::process::id()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let program = root.join("git-ignoring-input");
        std::fs::write(&program, "#!/bin/sh\nexit 0\n").unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
        let gate = Arc::new(TestPipeReaderGate::default());
        let provider = GitCliProvider::new(&program).with_pipe_reader_gate(gate);

        let error = provider
            .run_with_input_bounded(&root, &["apply"], b"required input", 1024)
            .unwrap_err();

        assert!(matches!(error, GitError::Io { .. }), "{error:?}");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn discovery_without_a_marker_does_not_invoke_git() {
        use std::os::unix::fs::PermissionsExt;

        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "runyte-git-marker-absence-{}-{nonce}",
            std::process::id()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let invoked = root.join("invoked");
        let program = root.join("failing-git");
        std::fs::write(
            &program,
            format!(
                "#!/bin/sh\nprintf invoked > '{}'\nexit 1\n",
                invoked.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();

        assert!(!has_git_marker_in(std::iter::once(root.as_path()), |_| false).unwrap());
        assert!(
            GitCliProvider::new(&program)
                .discover_with_marker_probe(&root, |_| Ok(false))
                .unwrap()
                .is_none()
        );
        assert!(!invoked.exists(), "Git ran despite there being no marker");

        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn repository_marker_probe_distinguishes_absent_present_and_invalid() {
        use std::os::unix::fs::symlink;

        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "runyte-git-marker-probe-{}-{nonce}",
            std::process::id()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let probe = || has_git_marker_in(std::iter::once(root.as_path()), |_| false);

        assert!(!probe().unwrap());
        std::fs::create_dir(root.join(".git")).unwrap();
        assert!(matches!(probe(), Err(GitError::Io { .. })));
        std::fs::write(root.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
        assert!(probe().unwrap());

        std::fs::remove_dir_all(root.join(".git")).unwrap();
        std::fs::write(root.join(".git"), "not a gitdir file\n").unwrap();
        assert!(matches!(probe(), Err(GitError::Io { .. })));
        std::fs::write(root.join(".git"), "gitdir: ../metadata/worktrees/linked\n").unwrap();
        assert!(probe().unwrap());

        std::fs::remove_file(root.join(".git")).unwrap();
        symlink("elsewhere", root.join(".git")).unwrap();
        assert!(matches!(probe(), Err(GitError::Io { .. })));

        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn shared_scratch_marker_is_a_ceiling_but_private_markers_remain_decisive() {
        use std::os::unix::fs::PermissionsExt;

        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "runyte-git-marker-ceiling-{}-{nonce}",
            std::process::id()
        ));
        let ceiling = root.join("shared");
        let private = ceiling.join("private");
        let workspace = private.join("workspace");
        std::fs::create_dir_all(ceiling.join(".git")).unwrap();
        std::fs::create_dir_all(&workspace).unwrap();
        let invoked = root.join("invoked");
        let program = root.join("failing-git");
        std::fs::write(
            &program,
            format!(
                "#!/bin/sh\nprintf invoked > '{}'\nexit 1\n",
                invoked.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
        let probe =
            |start: &Path| has_git_marker_in(start.ancestors(), |directory| directory == ceiling);
        let provider = GitCliProvider::new(&program);

        assert!(
            provider
                .discover_with_marker_probe(&workspace, probe)
                .unwrap()
                .is_none()
        );
        assert!(!invoked.exists(), "Git ran past the shared scratch ceiling");

        std::fs::create_dir(workspace.join(".git")).unwrap();
        assert!(matches!(
            provider.discover_with_marker_probe(&workspace, probe),
            Err(GitError::Io { .. })
        ));
        std::fs::remove_dir_all(workspace.join(".git")).unwrap();

        std::fs::create_dir(private.join(".git")).unwrap();
        std::fs::write(private.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
        std::fs::create_dir(workspace.join(".git")).unwrap();
        assert!(matches!(
            provider.discover_with_marker_probe(&workspace, probe),
            Err(GitError::Io { path, .. }) if path == workspace.join(".git")
        ));

        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn discovery_with_a_marker_propagates_git_failure() {
        use std::os::unix::fs::PermissionsExt;

        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "runyte-git-marker-failure-{}-{nonce}",
            std::process::id()
        ));
        std::fs::create_dir_all(root.join(".git")).unwrap();
        std::fs::write(root.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
        let program = root.join("failing-git");
        std::fs::write(
            &program,
            "#!/bin/sh\nprintf 'broken repository' >&2\nexit 1\n",
        )
        .unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();

        let error = GitCliProvider::new(&program).discover(&root).unwrap_err();
        assert!(matches!(error, GitError::Failed { .. }), "{error:?}");

        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn repository_discovery_rejects_empty_required_rev_parse_output() {
        use std::os::unix::fs::PermissionsExt;

        for empty_argument in ["--show-toplevel", "--git-dir", "--git-common-dir"] {
            let nonce = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let root = std::env::temp_dir().join(format!(
                "runyte-git-empty-discovery-{}-{nonce}",
                std::process::id()
            ));
            std::fs::create_dir_all(root.join(".git")).unwrap();
            std::fs::write(root.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
            let program = root.join("git-with-empty-discovery-field");
            std::fs::write(
                &program,
                format!(
                    "#!/bin/sh\n\
                     case \"$*\" in\n\
                       *{empty_argument}*) exit 0 ;;\n\
                       *--show-toplevel*) printf '%s\\n' '{}' ;;\n\
                       *--git-dir*) printf '.git\\n' ;;\n\
                       *--git-common-dir*) printf '.git\\n' ;;\n\
                     esac\n",
                    root.display()
                ),
            )
            .unwrap();
            std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();

            let error = GitCliProvider::new(&program).discover(&root).unwrap_err();
            assert!(
                matches!(error, GitError::Malformed { ref command, .. } if command.contains(empty_argument)),
                "{empty_argument} produced {error:?}"
            );

            std::fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn a_missing_git_reads_as_unavailable_rather_than_a_failure() {
        let provider = GitCliProvider::new("runyte-git-that-does-not-exist");
        let error = provider
            .run_text(Path::new("."), &["rev-parse", "--show-toplevel"])
            .unwrap_err();

        assert!(error.is_unavailable(), "{error}");
    }

    #[cfg(unix)]
    #[test]
    fn a_non_executable_git_reads_as_unavailable() {
        let root = std::env::temp_dir().join(format!(
            "runyte-non-executable-git-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let program = root.join("git");
        std::fs::write(&program, "not executable").unwrap();

        let error = GitCliProvider::new(&program)
            .run_text(&root, &["status"])
            .unwrap_err();

        assert!(error.is_unavailable(), "{error}");
        let not_directory = root.join("ordinary-file");
        std::fs::write(&not_directory, "content").unwrap();
        assert!(matches!(
            GitCliProvider::new("git").run_text(&not_directory, &["status"]),
            Err(GitError::Io { .. })
        ));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn discovery_looks_for_git_without_starting_it() {
        assert!(GitCliProvider::discover(Some(OsStr::new(""))).is_none());
        assert!(GitCliProvider::discover(None).is_none());
    }
}
