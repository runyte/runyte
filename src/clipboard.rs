// SPDX-License-Identifier: MPL-2.0

//! Operating-system clipboard access without a GUI event-loop dependency.
//!
//! Terminal editors commonly run in environments where linking a particular
//! display-server client is either impossible or undesirable. Runyte therefore
//! uses the platform's standard clipboard helper, trying the native choices in
//! deterministic order. Commands are invoked directly, never through a shell.
//!
//! Every helper runs through [`run_helper`], which never blocks the editor on
//! anything but the helper's own exit, and bounds even that. The write helpers
//! on Wayland and X11 — `wl-copy`, `xclip -in`, `xsel --input` — fork a process
//! that owns the selection for as long as the copied value lives, and that
//! process inherits the pipes given to the command Runyte spawned. Reading such
//! a pipe to end of file therefore waits for the clipboard's lifetime rather
//! than the command's: the editor stays frozen until something else takes the
//! selection over. Pipes are consequently drained on their own threads and are
//! only ever waited on with a deadline.

use std::{
    ffi::OsString,
    io::{self, Read, Write},
    process::{Child, Command, ExitStatus, Stdio},
    sync::mpsc::{self, Receiver, RecvTimeoutError},
    thread,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail};
#[cfg(unix)]
use std::os::unix::process::CommandExt;

const MAX_HELPER_STDERR_BYTES: usize = 1_024;
const MAX_CLIPBOARD_TEXT_BYTES: usize = 64 * 1024 * 1024;

/// How long a clipboard helper may take to exit before Runyte kills it.
///
/// The editor calls the clipboard synchronously from its event loop, so this is
/// also the longest a keystroke can stall. It has to clear a cold
/// `powershell.exe` start on Windows with room to spare, and stay short enough
/// that a wedged display server reads as an error rather than a hang.
const HELPER_TIMEOUT: Duration = Duration::from_secs(5);

/// How long a helper's pipes are drained for once it has already exited.
const HELPER_PIPE_GRACE: Duration = Duration::from_millis(250);

const HELPER_POLL_INTERVAL: Duration = Duration::from_millis(2);

/// Testable clipboard boundary used by the editor.
pub trait SystemClipboard: Send {
    fn read(&mut self) -> Result<String>;
    fn write(&mut self, text: &str) -> Result<()>;
}

/// Clipboard backed by the host platform's standard command-line helpers.
#[derive(Default)]
pub struct CommandClipboard;

impl SystemClipboard for CommandClipboard {
    fn read(&mut self) -> Result<String> {
        read_with_candidates(read_candidates())
    }

    fn write(&mut self, text: &str) -> Result<()> {
        write_with_candidates(write_candidates(), text)
    }
}

fn read_with_candidates(candidates: &[Candidate]) -> Result<String> {
    let mut failures = Vec::new();
    for candidate in candidates {
        match run_helper(
            candidate.program,
            &owned_arguments(candidate.args),
            None,
            Some(MAX_CLIPBOARD_TEXT_BYTES),
            HELPER_TIMEOUT,
        ) {
            Ok(output) if output.success => {
                return String::from_utf8(output.stdout)
                    .context("system clipboard did not contain UTF-8 text");
            }
            Ok(output) => failures.push(describe_failure(candidate.program, &output)),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => failures.push(format!("{}: {error}", candidate.program)),
        }
    }
    unavailable("read", &failures)
}

fn write_with_candidates(candidates: &[Candidate], text: &str) -> Result<()> {
    let mut failures = Vec::new();
    for candidate in candidates {
        // The helper's own stdout is discarded: keeping a pipe it can hand to a
        // forked selection owner would leave a reader thread parked for as long
        // as the clipboard holds this value.
        match run_helper(
            candidate.program,
            &owned_arguments(candidate.args),
            Some(text),
            None,
            HELPER_TIMEOUT,
        ) {
            Ok(output) if output.success => return Ok(()),
            Ok(output) => failures.push(describe_failure(candidate.program, &output)),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => failures.push(format!("{}: {error}", candidate.program)),
        }
    }
    unavailable("write", &failures)
}

fn owned_arguments(arguments: &[&str]) -> Vec<OsString> {
    arguments.iter().map(OsString::from).collect()
}

fn describe_failure(program: &str, output: &BoundedCommandOutput) -> String {
    let stderr = bounded_stderr(&output.stderr);
    if !stderr.is_empty() {
        return format!("{program}: {stderr}");
    }
    match output.code {
        Some(code) => format!("{program}: exited with status {code}"),
        None => format!("{program}: exited unsuccessfully"),
    }
}

#[derive(Clone, Copy)]
struct Candidate {
    program: &'static str,
    args: &'static [&'static str],
}

#[cfg(target_os = "macos")]
fn read_candidates() -> &'static [Candidate] {
    &[Candidate {
        program: "pbpaste",
        args: &[],
    }]
}

#[cfg(target_os = "macos")]
fn write_candidates() -> &'static [Candidate] {
    &[Candidate {
        program: "pbcopy",
        args: &[],
    }]
}

#[cfg(target_os = "windows")]
fn read_candidates() -> &'static [Candidate] {
    &[Candidate {
        program: "powershell.exe",
        args: &["-NoProfile", "-Command", "Get-Clipboard -Raw"],
    }]
}

#[cfg(target_os = "windows")]
fn write_candidates() -> &'static [Candidate] {
    &[Candidate {
        program: "powershell.exe",
        args: &["-NoProfile", "-Command", "$input | Set-Clipboard"],
    }]
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn read_candidates() -> &'static [Candidate] {
    &[
        Candidate {
            program: "wl-paste",
            args: &["--no-newline"],
        },
        Candidate {
            program: "xclip",
            args: &["-selection", "clipboard", "-out"],
        },
        Candidate {
            program: "xsel",
            args: &["--clipboard", "--output"],
        },
    ]
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn write_candidates() -> &'static [Candidate] {
    &[
        Candidate {
            program: "wl-copy",
            args: &[],
        },
        Candidate {
            program: "xclip",
            args: &["-selection", "clipboard", "-in"],
        },
        Candidate {
            program: "xsel",
            args: &["--clipboard", "--input"],
        },
    ]
}

fn unavailable<T>(operation: &str, failures: &[String]) -> Result<T> {
    if failures.is_empty() {
        bail!("cannot {operation} the system clipboard: install wl-clipboard, xclip, or xsel");
    }
    bail!(
        "cannot {operation} the system clipboard: {}",
        failures.join("; ")
    )
}

fn bounded_stderr(stderr: &[u8]) -> String {
    let text = String::from_utf8_lossy(stderr);
    let text = text.trim();
    if text.len() <= MAX_HELPER_STDERR_BYTES {
        return text.to_owned();
    }
    let mut end = MAX_HELPER_STDERR_BYTES;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &text[..end])
}

#[derive(Debug)]
struct BoundedCommandOutput {
    success: bool,
    code: Option<i32>,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

/// Runs one clipboard helper to completion under a deadline.
///
/// Nothing here waits on a pipe that a forked selection owner may still hold:
/// stdin, stdout, and stderr each get their own thread, the helper's exit is
/// polled against the deadline, and a helper that outlives the deadline is
/// killed. `stdout_limit` of `None` discards the helper's output rather than
/// giving it a pipe at all.
fn run_helper(
    program: &str,
    arguments: &[OsString],
    stdin_text: Option<&str>,
    stdout_limit: Option<usize>,
    timeout: Duration,
) -> io::Result<BoundedCommandOutput> {
    let mut command = Command::new(program);
    command
        .args(arguments)
        .stdin(if stdin_text.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(if stdout_limit.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stderr(Stdio::piped());
    #[cfg(unix)]
    command.process_group(0);
    let mut child = command.spawn()?;
    let deadline = Instant::now() + timeout;
    let outcome = (|| -> io::Result<BoundedCommandOutput> {
        let written = match (stdin_text, child.stdin.take()) {
            (Some(text), Some(mut stdin)) => {
                let text = text.to_owned();
                // The pipe closes when the thread drops it, which is what tells the
                // helper the clipboard value is complete.
                Some(detached(move || stdin.write_all(text.as_bytes())))
            }
            (Some(_), None) => {
                return Err(io::Error::other("clipboard helper stdin is unavailable"));
            }
            (None, _) => None,
        };
        let captured = match (stdout_limit, child.stdout.take()) {
            (Some(limit), Some(stdout)) => Some(detached(move || read_bounded(stdout, limit))),
            (Some(_), None) => {
                return Err(io::Error::other("clipboard helper stdout is unavailable"));
            }
            (None, _) => None,
        };
        let diagnostics = child.stderr.take().map(|stderr| {
            detached(move || read_and_discard_after_limit(stderr, MAX_HELPER_STDERR_BYTES))
        });

        let Some(status) = wait_until(&mut child, deadline)? else {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                format!(
                    "{program} did not exit within {} second(s)",
                    timeout.as_secs_f32()
                ),
            ));
        };

        if let Some(written) = written
            && let Err(error) = collect(written, deadline)
            && status.success()
        {
            // A helper that took less than the whole value yet reported success
            // would otherwise leave a silently truncated clipboard behind. When it
            // reported failure, its own diagnostics explain more than this does.
            return Err(error);
        }

        let grace = deadline.max(Instant::now() + HELPER_PIPE_GRACE);
        let stdout = match captured {
            Some(captured) => collect(captured, grace)?.ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::TimedOut,
                    format!("{program} output was still open after it exited"),
                )
            })?,
            None => Vec::new(),
        };

        // Diagnostics are only worth waiting for when there is a failure to
        // explain, and even then only briefly: on the success path the pipe belongs
        // to a selection owner that outlives this call by design.
        let stderr = if status.success() {
            Vec::new()
        } else {
            diagnostics
                .and_then(|diagnostics| collect(diagnostics, grace).ok().flatten())
                .unwrap_or_default()
        };

        Ok(BoundedCommandOutput {
            success: status.success(),
            code: status.code(),
            stdout,
            stderr,
        })
    })();
    if !matches!(&outcome, Ok(output) if output.success) {
        terminate_helper(&mut child);
    }
    outcome
}

fn terminate_helper(child: &mut Child) {
    #[cfg(unix)]
    {
        // Helpers get a private process group before spawn. Killing the group
        // on timeout also retires descendants that inherited the helper's
        // pipes; killing only the direct child can leave those descendants
        // running and their reader threads parked indefinitely.
        let process_group = -(child.id() as libc::pid_t);
        // SAFETY: a negative PID addresses exactly the process group created
        // for this child, and SIGKILL requires no userspace signal handler.
        let _ = unsafe { libc::kill(process_group, libc::SIGKILL) };
    }
    #[cfg(not(unix))]
    let _ = child.kill();
    let _ = child.wait();
}

/// Runs `work` on its own thread and hands back a receiver for its result.
///
/// The thread is never joined. A pipe held open by a forked selection owner
/// leaves one parked read behind, which ends as soon as that owner releases the
/// clipboard; joining it instead is what froze the editor.
fn detached<T: Send + 'static>(
    work: impl FnOnce() -> io::Result<T> + Send + 'static,
) -> Receiver<io::Result<T>> {
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let _ = sender.send(work());
    });
    receiver
}

/// `Ok(None)` means the thread was still working when the deadline passed.
fn collect<T>(receiver: Receiver<io::Result<T>>, deadline: Instant) -> io::Result<Option<T>> {
    match receiver.recv_timeout(remaining(deadline)) {
        Ok(Ok(value)) => Ok(Some(value)),
        Ok(Err(error)) => Err(error),
        Err(RecvTimeoutError::Timeout) => Ok(None),
        Err(RecvTimeoutError::Disconnected) => {
            Err(io::Error::other("clipboard helper pipe reader panicked"))
        }
    }
}

/// `Ok(None)` means the helper was still running when the deadline passed.
fn wait_until(child: &mut Child, deadline: Instant) -> io::Result<Option<ExitStatus>> {
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(Some(status));
        }
        let left = remaining(deadline);
        if left.is_zero() {
            return Ok(None);
        }
        thread::sleep(HELPER_POLL_INTERVAL.min(left));
    }
}

fn remaining(deadline: Instant) -> Duration {
    deadline.saturating_duration_since(Instant::now())
}

fn read_bounded(reader: impl Read, limit: usize) -> io::Result<Vec<u8>> {
    // One byte past the limit is enough to detect the overflow, and reading on
    // past it keeps the helper from blocking on a pipe nobody is draining.
    let kept = read_and_discard_after_limit(reader, limit.saturating_add(1))?;
    if kept.len() > limit {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("clipboard helper output exceeds {limit} byte(s)"),
        ));
    }
    Ok(kept)
}

fn read_and_discard_after_limit(mut reader: impl Read, limit: usize) -> io::Result<Vec<u8>> {
    let mut kept = Vec::with_capacity(limit.min(64 * 1024));
    let mut chunk = [0_u8; 8 * 1024];
    loop {
        let read = reader.read(&mut chunk)?;
        if read == 0 {
            break;
        }
        let remaining = limit.saturating_sub(kept.len());
        kept.extend_from_slice(&chunk[..read.min(remaining)]);
    }
    Ok(kept)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn a_silent_failure_is_reported_with_its_exit_status() {
        let quiet = BoundedCommandOutput {
            success: false,
            code: Some(3),
            stdout: Vec::new(),
            stderr: Vec::new(),
        };
        assert_eq!(
            describe_failure("wl-copy", &quiet),
            "wl-copy: exited with status 3"
        );

        let noisy = BoundedCommandOutput {
            success: false,
            code: None,
            stdout: Vec::new(),
            stderr: b"  no display  ".to_vec(),
        };
        assert_eq!(describe_failure("wl-copy", &noisy), "wl-copy: no display");
    }

    /// `wl-copy`, `xclip -in`, and `xsel --input` all fork a process that owns
    /// the selection until something else takes the clipboard over, and it
    /// inherits the pipes of the command Runyte spawned. Waiting for those
    /// pipes to reach end of file froze the editor after every yank.
    #[cfg(unix)]
    #[test]
    fn a_forked_selection_owner_does_not_freeze_the_write() {
        static CANDIDATES: &[Candidate] = &[Candidate {
            program: "sh",
            args: &["-c", "cat >/dev/null; sleep 10 & exit 0"],
        }];

        let (sender, receiver) = mpsc::channel();
        thread::spawn(move || {
            let _ = sender.send(write_with_candidates(CANDIDATES, "yanked"));
        });
        let outcome = receiver
            .recv_timeout(Duration::from_secs(5))
            .expect("the write outlived the helper it spawned");
        assert!(outcome.is_ok(), "{outcome:?}");
    }

    #[cfg(unix)]
    #[test]
    fn a_helper_that_never_exits_is_killed_rather_than_waited_on() {
        let started = Instant::now();
        let error = run_helper(
            "sh",
            &owned_arguments(&["-c", "sleep 30"]),
            None,
            Some(64),
            Duration::from_millis(200),
        )
        .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[cfg(unix)]
    #[test]
    fn timing_out_a_helper_also_kills_its_descendants() {
        let root = std::env::temp_dir().join(format!(
            "runyte-clipboard-descendants-{}-{:?}",
            std::process::id(),
            Instant::now()
        ));
        fs::create_dir_all(&root).unwrap();
        let marker = root.join("leaked");
        let error = run_helper(
            "sh",
            &[
                "-c".into(),
                "(sleep 0.4; printf leaked > \"$1\") & wait".into(),
                "sh".into(),
                marker.as_os_str().to_owned(),
            ],
            None,
            Some(64),
            Duration::from_millis(100),
        )
        .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        std::thread::sleep(Duration::from_millis(500));
        assert!(!marker.exists(), "a timed-out helper descendant survived");
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn a_successful_parent_with_stuck_output_cleans_up_its_descendant() {
        let root = std::env::temp_dir().join(format!(
            "runyte-clipboard-stuck-output-{}-{:?}",
            std::process::id(),
            Instant::now()
        ));
        fs::create_dir_all(&root).unwrap();
        let marker = root.join("leaked");
        let error = run_helper(
            "sh",
            &[
                "-c".into(),
                "(sleep 0.5; printf leaked > \"$1\") & exit 0".into(),
                "sh".into(),
                marker.as_os_str().to_owned(),
            ],
            None,
            Some(64),
            Duration::from_millis(100),
        )
        .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        std::thread::sleep(Duration::from_millis(600));
        assert!(!marker.exists(), "a stuck output descendant survived");
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn helper_input_is_delivered_and_output_is_bounded() {
        let output = run_helper(
            "sh",
            &owned_arguments(&["-c", "cat"]),
            Some("yanked"),
            Some(64),
            HELPER_TIMEOUT,
        )
        .unwrap();
        assert!(output.success);
        assert_eq!(output.stdout, b"yanked");

        let error = run_helper(
            "sh",
            &owned_arguments(&["-c", "printf yanked"]),
            None,
            Some(3),
            HELPER_TIMEOUT,
        )
        .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    }

    #[cfg(unix)]
    #[test]
    fn a_missing_helper_is_skipped_and_a_failing_one_is_reported() {
        static CANDIDATES: &[Candidate] = &[
            Candidate {
                program: "runyte-clipboard-helper-that-does-not-exist",
                args: &[],
            },
            Candidate {
                program: "sh",
                args: &["-c", "echo 'no display' >&2; exit 3"],
            },
        ];

        let error = write_with_candidates(CANDIDATES, "yanked")
            .unwrap_err()
            .to_string();
        assert!(error.contains("no display"), "{error}");
        assert!(!error.contains("does-not-exist"), "{error}");
    }

    #[cfg(unix)]
    #[test]
    fn reading_uses_the_first_helper_that_is_installed() {
        static CANDIDATES: &[Candidate] = &[
            Candidate {
                program: "runyte-clipboard-helper-that-does-not-exist",
                args: &[],
            },
            Candidate {
                program: "sh",
                args: &["-c", "printf 'from the clipboard'"],
            },
        ];

        assert_eq!(
            read_with_candidates(CANDIDATES).unwrap(),
            "from the clipboard"
        );
    }
}
