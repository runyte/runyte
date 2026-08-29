// SPDX-License-Identifier: MPL-2.0

//! The pseudoterminal one child process runs on.
//!
//! The only place in Runyte that forks a process onto a tty. Everything above
//! it sees bytes in, bytes out, a size, and an exit — never a file descriptor.
//!
//! Unix only. Windows needs ConPTY, which is a second implementation of the
//! hardest part of this file; `context/issues/windows_support.md` already
//! records that Runyte disables a feature there rather than shipping an
//! unsound one.

use std::{
    ffi::{CString, OsStr},
    io,
    os::unix::{
        ffi::OsStrExt,
        io::{AsRawFd, FromRawFd, OwnedFd, RawFd},
        process::CommandExt,
    },
    path::Path,
    process::{Child, Command, Stdio},
    sync::mpsc,
    thread,
};

/// How much of one read is handed upward at a time.
const READ_CHUNK: usize = 64 * 1024;
const WRITE_CHUNK: usize = 16 * 1024;
const INPUT_QUEUE: usize = 8;
pub const MAX_INPUT_BYTES: usize = 1024 * 1024;

/// What a running child produces.
#[derive(Debug)]
pub enum PtyEvent {
    Output(Vec<u8>),
    /// The child is gone. Carries its status code when one was reported.
    Exited(Option<i32>),
}

/// A child process attached to a pseudoterminal.
pub struct Pty {
    master: OwnedFd,
    input: mpsc::SyncSender<Vec<u8>>,
    child: Child,
}

/// Owns a spawned child until every fallible PTY setup step has succeeded.
///
/// `Child` does not terminate or reap its process when dropped. Descriptor
/// duplication and either background-thread spawn can still fail after the
/// program exists, so ordinary `?` unwinding needs an owner that does both.
struct SpawnedChild {
    child: Option<Child>,
}

impl SpawnedChild {
    fn new(child: Child) -> Self {
        Self { child: Some(child) }
    }

    fn id(&self) -> u32 {
        self.child.as_ref().expect("spawn guard is armed").id()
    }

    fn disarm(mut self) -> Child {
        self.child.take().expect("spawn guard is armed")
    }
}

impl Drop for SpawnedChild {
    fn drop(&mut self) {
        if let Some(child) = self.child.as_mut() {
            terminate_child(child);
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SpawnCheckpoint {
    ChildOwned,
    ReaderDuplicated,
    WriterDuplicated,
    WriterStarted,
}

impl std::fmt::Debug for Pty {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Pty")
            .field("pid", &self.child.id())
            .finish_non_exhaustive()
    }
}

impl Pty {
    /// Runs `program` with `arguments` on a new pseudoterminal.
    ///
    /// `events` receives everything the child writes and, once, its exit.
    pub fn spawn(
        program: &OsStr,
        arguments: &[String],
        directory: &Path,
        columns: u16,
        rows: u16,
        events: impl Fn(PtyEvent) + Send + 'static,
    ) -> io::Result<Self> {
        Self::spawn_with_checkpoints(
            program,
            arguments,
            directory,
            columns,
            rows,
            events,
            |_, _| Ok(()),
        )
    }

    fn spawn_with_checkpoints(
        program: &OsStr,
        arguments: &[String],
        directory: &Path,
        columns: u16,
        rows: u16,
        events: impl Fn(PtyEvent) + Send + 'static,
        mut checkpoint: impl FnMut(SpawnCheckpoint, u32) -> io::Result<()>,
    ) -> io::Result<Self> {
        let (master, slave) = open_pair(columns, rows)?;
        let slave_descriptor = slave.as_raw_fd();

        let mut command = Command::new(program);
        command.args(arguments);
        command.current_dir(directory);
        command.stdin(Stdio::null());
        command.stdout(Stdio::null());
        command.stderr(Stdio::null());
        // A child inheriting the outer terminal's identity would advertise
        // capabilities this emulator does not have. Name what is actually
        // implemented instead.
        command.env("TERM", "xterm-256color");
        command.env("COLORTERM", "truecolor");
        command.env_remove("TERM_PROGRAM");
        command.env_remove("TERM_PROGRAM_VERSION");
        // Nothing below runs in the parent: `pre_exec` is on the child side of
        // the fork, where only async-signal-safe calls are allowed. Opening the
        // already-open slave here becomes the controlling terminal only after
        // `setsid`; opening it with `O_NOCTTY` in `openpty` keeps the parent
        // from acquiring it. Keeping both endpoints open before the fork also
        // lets macOS apply the initial window size to the slave, as its PTY API
        // requires.
        unsafe {
            command.pre_exec(move || {
                if libc::setsid() < 0 {
                    return Err(io::Error::last_os_error());
                }
                if libc::ioctl(slave_descriptor, libc::TIOCSCTTY as _, 0) < 0 {
                    return Err(io::Error::last_os_error());
                }
                for descriptor in 0..3 {
                    if libc::dup2(slave_descriptor, descriptor) < 0 {
                        return Err(io::Error::last_os_error());
                    }
                }
                if slave_descriptor > 2 {
                    libc::close(slave_descriptor);
                }
                Ok(())
            });
        }
        let child = command.spawn()?;
        let child = SpawnedChild::new(child);
        checkpoint(SpawnCheckpoint::ChildOwned, child.id())?;
        // The child has duplicated this endpoint onto stdin/stdout/stderr.
        // Closing the parent's copy is what lets the reader observe EOF when
        // the child and all of its descendants finally close theirs.
        drop(slave);

        let (input, pending) = mpsc::sync_channel::<Vec<u8>>(INPUT_QUEUE);
        let reader = duplicate(master.as_raw_fd())?;
        checkpoint(SpawnCheckpoint::ReaderDuplicated, child.id())?;
        let writer = duplicate(master.as_raw_fd())?;
        checkpoint(SpawnCheckpoint::WriterDuplicated, child.id())?;

        // Writing on the caller's thread would let a child that has stopped
        // reading — a paused pager, a program waiting on something else —
        // block the editor's event loop. The queue is what keeps a keystroke
        // from ever doing that.
        thread::Builder::new()
            .name("runyte-pty-write".to_owned())
            .spawn(move || {
                let writer = writer;
                while let Ok(bytes) = pending.recv() {
                    for chunk in bytes.chunks(WRITE_CHUNK) {
                        if write_all(writer.as_raw_fd(), chunk).is_err() {
                            return;
                        }
                    }
                }
            })?;
        checkpoint(SpawnCheckpoint::WriterStarted, child.id())?;

        thread::Builder::new()
            .name("runyte-pty-read".to_owned())
            .spawn(move || {
                let reader = reader;
                let mut buffer = vec![0_u8; READ_CHUNK];
                loop {
                    let read = unsafe {
                        libc::read(reader.as_raw_fd(), buffer.as_mut_ptr().cast(), buffer.len())
                    };
                    match read {
                        // Zero is a clean end of file. `EIO` is what Linux
                        // reports when the last slave closes, which is the
                        // same event by another name.
                        0 => break,
                        count if count > 0 => {
                            events(PtyEvent::Output(buffer[..count as usize].to_vec()));
                        }
                        _ => {
                            let error = io::Error::last_os_error();
                            if error.kind() == io::ErrorKind::Interrupted {
                                continue;
                            }
                            break;
                        }
                    }
                }
                events(PtyEvent::Exited(None));
            })?;

        Ok(Self {
            master,
            input,
            child: child.disarm(),
        })
    }

    /// Queues bytes for the child. Never blocks the caller.
    pub fn write(&self, bytes: Vec<u8>) -> bool {
        if bytes.is_empty() {
            return true;
        }
        if bytes.len() > MAX_INPUT_BYTES {
            return false;
        }
        self.input.try_send(bytes).is_ok()
    }

    pub fn resize(&self, columns: u16, rows: u16) -> io::Result<()> {
        set_size(self.master.as_raw_fd(), columns, rows)
    }

    /// Asks the child's process group to end, then ends it.
    ///
    /// `SIGHUP` first because that is what a closing terminal sends and what a
    /// shell knows how to act on; `SIGKILL` after, because a pane that has
    /// been closed must not leave a process holding the pty open.
    pub fn terminate(&mut self) {
        terminate_child(&mut self.child);
    }

    /// Reports the child's status if it has already finished, without waiting.
    pub fn finished(&mut self) -> Option<Option<i32>> {
        match self.child.try_wait() {
            Ok(Some(status)) => Some(status.code()),
            Ok(None) => None,
            Err(_) => Some(None),
        }
    }
}

fn terminate_child(child: &mut Child) {
    terminate_child_with(child, |process_group, signal| {
        // SAFETY: `terminate_child_with` calls this only while the direct
        // child still anchors its private process-group identity.
        unsafe {
            libc::kill(process_group, signal);
        }
    });
}

fn terminate_child_with(child: &mut Child, mut signal_group: impl FnMut(libc::pid_t, libc::c_int)) {
    match child.try_wait() {
        // `try_wait` retains this result for subsequent calls, but it has
        // already reaped the child and released the numeric PID. Never use
        // that stale number as a process-group identity.
        Ok(Some(_)) => return,
        Ok(None) => {}
        Err(error) => {
            crate::log_warn!(
                "terminal",
                "could not prove PTY child ownership before termination";
                "pid" => child.id(),
                "error" => error,
            );
            return;
        }
    }
    let process_group = -(child.id() as libc::pid_t);
    signal_group(process_group, libc::SIGHUP);
    signal_group(process_group, libc::SIGKILL);
    let _ = child.wait();
}

impl Drop for Pty {
    fn drop(&mut self) {
        self.terminate();
    }
}

fn open_pair(columns: u16, rows: u16) -> io::Result<(OwnedFd, OwnedFd)> {
    let mut master = -1;
    let mut slave = -1;
    // Apple and several BSD libc declarations expose these inputs as mutable
    // pointers, while glibc exposes them as const. Mutable pointers satisfy
    // both declarations; `openpty` only reads the values on either platform.
    let mut size = window_size(columns, rows);
    let result = unsafe {
        libc::openpty(
            &mut master,
            &mut slave,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::addr_of_mut!(size),
        )
    };
    if result < 0 {
        return Err(io::Error::last_os_error());
    }
    let master = unsafe { OwnedFd::from_raw_fd(master) };
    let slave = unsafe { OwnedFd::from_raw_fd(slave) };
    // Neither endpoint may survive into an unrelated child: an editor that
    // spawns a language server while a terminal is open would otherwise hand
    // it descriptors it has no business holding. `dup2` clears this flag on
    // the child's three standard descriptors.
    for descriptor in [master.as_raw_fd(), slave.as_raw_fd()] {
        if unsafe { libc::fcntl(descriptor, libc::F_SETFD, libc::FD_CLOEXEC) } < 0 {
            return Err(io::Error::last_os_error());
        }
    }
    Ok((master, slave))
}

fn duplicate(descriptor: RawFd) -> io::Result<OwnedFd> {
    let copy = unsafe { libc::fcntl(descriptor, libc::F_DUPFD_CLOEXEC, 0) };
    if copy < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(unsafe { OwnedFd::from_raw_fd(copy) })
}

fn set_size(descriptor: RawFd, columns: u16, rows: u16) -> io::Result<()> {
    let size = window_size(columns, rows);
    let result = unsafe { libc::ioctl(descriptor, libc::TIOCSWINSZ as _, &size) };
    if result < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

fn window_size(columns: u16, rows: u16) -> libc::winsize {
    libc::winsize {
        ws_row: rows.max(1),
        ws_col: columns.max(1),
        ws_xpixel: 0,
        ws_ypixel: 0,
    }
}

fn write_all(descriptor: RawFd, bytes: &[u8]) -> io::Result<()> {
    let mut written = 0;
    while written < bytes.len() {
        let count = unsafe {
            libc::write(
                descriptor,
                bytes[written..].as_ptr().cast(),
                bytes.len() - written,
            )
        };
        if count > 0 {
            written += count as usize;
            continue;
        }
        let error = io::Error::last_os_error();
        if error.kind() == io::ErrorKind::Interrupted {
            continue;
        }
        return Err(error);
    }
    Ok(())
}

/// The program a bare terminal request runs.
///
/// `$SHELL` is the person's stated choice, so it wins. `/bin/sh` exists on
/// every Unix and is the fallback that cannot fail to be a shell.
pub fn default_shell() -> std::ffi::OsString {
    match std::env::var_os("SHELL") {
        Some(shell) if !shell.is_empty() => shell,
        _ => std::ffi::OsString::from("/bin/sh"),
    }
}

/// Whether a path names something this process could execute.
pub fn is_executable(path: &Path) -> bool {
    let Ok(path) = CString::new(path.as_os_str().as_bytes()) else {
        return false;
    };
    unsafe { libc::access(path.as_ptr(), libc::X_OK) == 0 }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        sync::{Arc, Mutex},
        time::{Duration, Instant},
    };

    /// A running child, everything it has written so far, and whether it has
    /// ended. The `Pty` comes back so the caller keeps it alive: dropping one
    /// kills its child.
    struct Running {
        output: Arc<Mutex<Vec<u8>>>,
        exited: Arc<Mutex<bool>>,
        pty: Pty,
    }

    fn collect(program: &str, arguments: &[&str]) -> Running {
        let output = Arc::new(Mutex::new(Vec::new()));
        let exited = Arc::new(Mutex::new(false));
        let sink = Arc::clone(&output);
        let done = Arc::clone(&exited);
        let arguments = arguments
            .iter()
            .map(|value| (*value).to_owned())
            .collect::<Vec<_>>();
        let pty = Pty::spawn(
            OsStr::new(program),
            &arguments,
            Path::new("/"),
            40,
            10,
            move |event| match event {
                PtyEvent::Output(bytes) => sink.lock().unwrap().extend_from_slice(&bytes),
                PtyEvent::Exited(_) => *done.lock().unwrap() = true,
            },
        )
        .expect("a pty can be opened in a test environment");
        Running {
            output,
            exited,
            pty,
        }
    }

    fn wait_until(deadline: Duration, mut ready: impl FnMut() -> bool) -> bool {
        let start = Instant::now();
        while start.elapsed() < deadline {
            if ready() {
                return true;
            }
            thread::sleep(Duration::from_millis(5));
        }
        ready()
    }

    fn process_group_exists(pid: u32) -> bool {
        let result = unsafe { libc::kill(-(pid as libc::pid_t), 0) };
        result == 0 || io::Error::last_os_error().kind() == io::ErrorKind::PermissionDenied
    }

    #[test]
    fn every_post_spawn_setup_failure_terminates_and_reaps_the_child() {
        for failed_at in [
            SpawnCheckpoint::ChildOwned,
            SpawnCheckpoint::ReaderDuplicated,
            SpawnCheckpoint::WriterDuplicated,
            SpawnCheckpoint::WriterStarted,
        ] {
            let observed_pid = Arc::new(Mutex::new(None));
            let captured_pid = Arc::clone(&observed_pid);
            let arguments = vec!["-c".to_owned(), "while :; do sleep 1; done".to_owned()];
            let error = Pty::spawn_with_checkpoints(
                OsStr::new("/bin/sh"),
                &arguments,
                Path::new("/"),
                40,
                10,
                |_| {},
                move |checkpoint, pid| {
                    if checkpoint == failed_at {
                        *captured_pid.lock().unwrap() = Some(pid);
                        return Err(io::Error::other(format!(
                            "injected failure at {checkpoint:?}"
                        )));
                    }
                    Ok(())
                },
            )
            .expect_err("the selected setup checkpoint fails");

            assert_eq!(error.kind(), io::ErrorKind::Other);
            let pid = observed_pid
                .lock()
                .unwrap()
                .expect("the spawned child identity was observed");
            assert!(
                wait_until(Duration::from_secs(1), || !process_group_exists(pid)),
                "post-spawn failure at {failed_at:?} left process group {pid} alive"
            );
        }
    }

    #[test]
    fn completed_child_teardown_never_signals_a_reusable_process_group() {
        let mut running = collect("/bin/sh", &["-c", "exit 0"]);
        assert!(wait_until(Duration::from_secs(5), || running
            .pty
            .finished()
            .is_some()));

        let mut signals = Vec::new();
        terminate_child_with(&mut running.pty.child, |group, signal| {
            signals.push((group, signal));
        });

        assert!(signals.is_empty(), "reaped child triggered {signals:?}");
    }

    #[test]
    fn running_child_teardown_still_signals_and_reaps_its_private_group() {
        let mut running = collect("/bin/sh", &["-c", "while :; do sleep 1; done"]);
        let pid = running.pty.child.id();
        let mut signals = Vec::new();

        terminate_child_with(&mut running.pty.child, |group, signal| {
            signals.push((group, signal));
            // SAFETY: the unreaped child still anchors this private group.
            unsafe {
                libc::kill(group, signal);
            }
        });

        let process_group = -(pid as libc::pid_t);
        assert_eq!(
            signals,
            [
                (process_group, libc::SIGHUP),
                (process_group, libc::SIGKILL),
            ]
        );
        assert!(!process_group_exists(pid));
    }

    #[test]
    fn a_child_writes_to_the_pty_and_then_exits() {
        // Keep the slave open until the reader has observed the output. Under
        // a saturated macOS runner an immediate child exit can surface the PTY
        // hangup before the background reader is scheduled, making the fixture
        // lose the bytes it was meant to test. One input line releases this
        // finite child, so the test still covers output followed by exit.
        let running = collect("/bin/sh", &["-c", "printf 'hello\\n'; read reply"]);
        assert!(wait_until(Duration::from_secs(5), || {
            String::from_utf8_lossy(&running.output.lock().unwrap()).contains("hello")
        }));
        assert!(running.pty.write(b"continue\n".to_vec()));
        assert!(wait_until(Duration::from_secs(5), || *running
            .exited
            .lock()
            .unwrap()));
    }

    #[test]
    fn a_child_sees_the_size_the_pty_was_opened_with() {
        // `stty` reads the controlling terminal, so its answer is the child's
        // own view rather than anything this process told it.
        let running = collect("/bin/sh", &["-c", "stty size"]);
        assert!(wait_until(Duration::from_secs(5), || {
            String::from_utf8_lossy(&running.output.lock().unwrap()).contains("10 40")
        }));
    }

    #[test]
    fn input_reaches_the_child() {
        let running = collect("/bin/cat", &[]);
        running.pty.write(b"ping\n".to_vec());
        assert!(wait_until(Duration::from_secs(5), || {
            String::from_utf8_lossy(&running.output.lock().unwrap()).contains("ping")
        }));
    }
}
