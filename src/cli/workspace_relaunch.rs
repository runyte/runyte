// SPDX-License-Identifier: MPL-2.0

//! Replaces a finished ide editor with one rooted in another workspace.
//!
//! This runs after the runtime and the log have shut down, exactly where the
//! process would otherwise exit. On Unix the process image is replaced, so the
//! launching shell, a `runyte()` wrapper and the desktop's view of the process
//! all carry on undisturbed. Windows has no equivalent: a terminal editor waits
//! for its successor so the console stays owned, and a window hands over and
//! closes.

use anyhow::{Context, Result};
use std::{ffi::OsString, path::PathBuf, process::Command};

pub(super) struct Relaunch {
    pub(super) arguments: Vec<OsString>,
    pub(super) directory: PathBuf,
    #[cfg_attr(unix, allow(dead_code))]
    pub(super) window: bool,
}

impl Relaunch {
    pub(super) fn replace_process(self) -> Result<()> {
        let executable = std::env::current_exe().context("cannot find the Runyte executable")?;
        let mut command = Command::new(executable);
        command.args(&self.arguments).current_dir(&self.directory);
        let directory = self.directory.display().to_string();
        let failure = || format!("cannot open the workspace at {directory}");
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            reap_ended_children();
            Err(command.exec()).with_context(failure)
        }
        #[cfg(not(unix))]
        {
            if self.window {
                command.spawn().with_context(failure)?;
                return Ok(());
            }
            let status = command.status().with_context(failure)?;
            std::process::exit(status.code().unwrap_or(1));
        }
    }
}

/// Collects the children the runtime killed on its way down. A process that
/// outlives them would simply exit and leave them to init; this one continues
/// under the same PID, and its successor has no record of them to wait for.
#[cfg(unix)]
fn reap_ended_children() {
    use std::time::{Duration, Instant};
    let deadline = Instant::now() + Duration::from_millis(250);
    loop {
        // SAFETY: a null status pointer is permitted, and WNOHANG never blocks.
        match unsafe { libc::waitpid(-1, std::ptr::null_mut(), libc::WNOHANG) } {
            0 if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(5)),
            pid if pid > 0 => {}
            _ => return,
        }
    }
}
