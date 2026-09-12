// SPDX-License-Identifier: MPL-2.0

//! Bounded plain-text shell filters. The workspace host owns admission and results.

use anyhow::{Result, bail, ensure};
use std::{
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

pub(crate) const MAX_BYTES: usize = 8 * 1024 * 1024;
const STDERR_BYTES: usize = 16 * 1024;

/// An opaque result delivered only to the workspace that started the job.
#[derive(Debug)]
pub struct Completion(pub(crate) Result<Vec<String>, String>);

pub(crate) fn run(
    command: &str,
    directory: &Path,
    inputs: Vec<String>,
    cancel: Arc<AtomicBool>,
) -> Completion {
    Completion(
        run_inner(command, directory, inputs, &cancel, Duration::from_secs(30))
            .map_err(|e| format!("{e:#}")),
    )
}

fn run_inner(
    command: &str,
    directory: &Path,
    inputs: Vec<String>,
    cancel: &AtomicBool,
    timeout: Duration,
) -> Result<Vec<String>> {
    let deadline = Instant::now() + timeout;
    let mut remaining = MAX_BYTES;
    let mut outputs = Vec::with_capacity(inputs.len());
    for input in inputs {
        ensure!(!cancel.load(Ordering::Acquire), "pipe cancelled");
        ensure!(Instant::now() < deadline, "pipe timed out");
        let output = invoke(
            Path::new("/bin/sh"),
            command,
            directory,
            input.as_bytes(),
            cancel,
            deadline,
            remaining,
        )?;
        remaining -= output.len();
        outputs.push(
            String::from_utf8(output)
                .map_err(|_| anyhow::anyhow!("pipe stdout is not valid UTF-8"))?,
        );
    }
    Ok(outputs)
}

#[cfg(unix)]
fn invoke(
    shell: &Path,
    command: &str,
    directory: &Path,
    input: &[u8],
    cancel: &AtomicBool,
    deadline: Instant,
    limit: usize,
) -> Result<Vec<u8>> {
    use crate::process_group::{self, GroupAnchor, Site};
    use std::{
        io::{ErrorKind, Read, Write},
        os::{fd::AsRawFd, unix::process::CommandExt},
        process::{Command, Stdio},
    };
    let mut child = Command::new(shell)
        .args(["-c", command])
        .current_dir(directory)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0)
        .spawn()?;
    process_group::record_spawn("pipe", "shell filter", child.id());
    // Never reap until the group has been cleaned up: even successful shells may
    // leave descendants holding the output pipes. All I/O stays nonblocking.
    let group = process_group::claim_anchored_group(
        Site::new("pipe", "filter cleanup"),
        child.id() as libc::pid_t,
        GroupAnchor::RunningLeader,
    );
    let mut errors = Vec::new();
    let mut error_truncated = false;
    let result = (|| -> Result<Vec<u8>> {
        let mut stdin = child.stdin.take();
        let mut stdout = child.stdout.take().expect("piped stdout");
        let mut stderr = child.stderr.take().expect("piped stderr");
        for fd in [
            stdin.as_ref().unwrap().as_raw_fd(),
            stdout.as_raw_fd(),
            stderr.as_raw_fd(),
        ] {
            // SAFETY: these are live, exclusively owned child pipe descriptors.
            let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
            if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0
            {
                return Err(std::io::Error::last_os_error().into());
            }
        }
        let mut written = 0;
        let mut output = Vec::new();
        let mut exited = None;
        loop {
            ensure!(!cancel.load(Ordering::Acquire), "pipe cancelled");
            ensure!(Instant::now() < deadline, "pipe timed out");
            if let Some(writer) = stdin.as_mut() {
                match writer.write(&input[written..input.len().min(written + 65536)]) {
                    Ok(n) => written += n,
                    Err(e)
                        if e.kind() == ErrorKind::WouldBlock
                            || e.kind() == ErrorKind::Interrupted => {}
                    Err(e) if e.kind() == ErrorKind::BrokenPipe => {
                        stdin = None;
                    }
                    Err(e) => return Err(e.into()),
                }
                if written == input.len() {
                    stdin = None;
                }
            }
            let mut stdout_drained = false;
            for (reader, target, maximum, truncate) in [
                (&mut stdout as &mut dyn Read, &mut output, limit, false),
                (
                    &mut stderr as &mut dyn Read,
                    &mut errors,
                    STDERR_BYTES,
                    true,
                ),
            ] {
                // Bound each turn even if a descendant continuously floods a pipe.
                for _ in 0..64 {
                    let mut bytes = [0; 8192];
                    match reader.read(&mut bytes) {
                        Ok(0) => {
                            if !truncate {
                                stdout_drained = true;
                            }
                            break;
                        }
                        Ok(n) => {
                            let take = n.min(maximum.saturating_sub(target.len()));
                            target.extend_from_slice(&bytes[..take]);
                            if take < n {
                                if !truncate {
                                    bail!("pipe stdout exceeds the 8 MiB job limit");
                                }
                                error_truncated = true;
                            }
                        }
                        Err(e) if e.kind() == ErrorKind::WouldBlock => {
                            if !truncate {
                                stdout_drained = true;
                            }
                            break;
                        }
                        Err(e) if e.kind() == ErrorKind::Interrupted => continue,
                        Err(e) => return Err(e.into()),
                    }
                }
            }
            if let Some(status) = exited {
                if stdout_drained {
                    let status: std::process::ExitStatus = status;
                    ensure!(status.success(), "pipe exited with {status}");
                    return Ok(output);
                }
            } else if let Some(status) = process_group::completed_without_reaping(&child)? {
                group.signal(libc::SIGKILL);
                stdin = None;
                exited = Some(status);
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    })();
    group.signal(libc::SIGKILL);
    let reaped = child.wait();
    reaped?;
    result.map_err(|error| {
        if errors.is_empty() {
            error
        } else {
            anyhow::anyhow!(
                "{error}: {}{}",
                String::from_utf8_lossy(&errors),
                if error_truncated {
                    " [stderr truncated]"
                } else {
                    ""
                }
            )
        }
    })
}

#[cfg(not(unix))]
fn invoke(
    _: &Path,
    _: &str,
    _: &Path,
    _: &[u8],
    _: &AtomicBool,
    _: Instant,
    _: usize,
) -> Result<Vec<u8>> {
    bail!("shell pipes require Unix")
}

#[cfg(all(test, unix))]
#[path = "pipe/tests.rs"]
mod tests;
