// SPDX-License-Identifier: MPL-2.0

//! Overlapped stdio and job ownership; no runtime blocking-pool pipe workers.
use super::*;
use crate::windows_process::overlapped::pipe;
use std::{io, process::Command};
use tokio::task::AbortHandle;

/// Both connection teardown and runtime teardown own a route to cleanup. The
/// monitor's guard is constructed before spawning so even an unpolled future
/// destroys the job when its runtime stops.
#[derive(Debug)]
pub(super) struct Native(Arc<Ownership>);

struct Ownership {
    child: Mutex<Option<crate::windows_process::Child>>,
    tasks: Vec<AbortHandle>,
}

impl std::fmt::Debug for Ownership {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NativeLspOwnership").finish_non_exhaustive()
    }
}

impl Ownership {
    fn finished(&self) -> bool {
        let mut child = self.child.lock().unwrap_or_else(|e| e.into_inner());
        child
            .as_mut()
            .is_none_or(|child| matches!(child.try_wait(), Ok(Some(_))))
    }

    fn stop(&self) {
        for task in &self.tasks {
            task.abort();
        }
        let child = self.child.lock().unwrap_or_else(|e| e.into_inner()).take();
        if let Some(mut child) = child {
            let _ = child.kill();
            // Windows has no Unix-style reap requirement. Closing the job
            // finishes ownership without a blocking wait on the runtime.
        }
    }
}

impl Native {
    pub(super) fn finished(&self) -> bool {
        self.0.finished()
    }
}

impl Drop for Native {
    fn drop(&mut self) {
        self.0.stop();
    }
}

struct Monitor {
    owner: Arc<Ownership>,
    armed: bool,
}
impl Drop for Monitor {
    fn drop(&mut self) {
        if self.armed {
            self.owner.stop();
        }
    }
}

pub(super) fn spawn(
    key: String,
    generation: u64,
    program: &Path,
    arguments: &[String],
    root: &Path,
    inbox: mpsc::Sender<(String, u64, Incoming)>,
) -> io::Result<Connection> {
    let mut command = Command::new(program);
    command.args(arguments).current_dir(root);
    spawn_command(key, generation, &command, inbox)
}

fn spawn_command(
    key: String,
    generation: u64,
    command: &Command,
    inbox: mpsc::Sender<(String, u64, Incoming)>,
) -> io::Result<Connection> {
    let (stdin, child_stdin) = pipe(false)?;
    let (stdout, child_stdout) = pipe(true)?;
    let (stderr, child_stderr) = pipe(true)?;
    let child = crate::windows_process::spawn_with_stdio(
        command,
        [child_stdin, child_stdout, child_stderr],
    )?;
    let mut connection = connect(key, generation, stdout, stdin, inbox);
    let task = tokio::spawn(drain_stderr(stderr, Arc::clone(&connection.stderr)));
    connection.tasks.push(task.abort_handle());
    let owner = Arc::new(Ownership {
        child: Mutex::new(Some(child)),
        tasks: connection.tasks.clone(),
    });
    let guard = Monitor {
        owner: Arc::clone(&owner),
        armed: true,
    };
    let monitor = tokio::spawn(async move {
        let mut guard = guard;
        loop {
            if guard.owner.finished() {
                // try_wait has terminated owned descendants. Let stdout and
                // stderr drain their remaining bytes, independent of inbox load.
                guard.armed = false;
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    });
    connection.tasks.push(monitor.abort_handle());
    connection.native = Some(Native(owner));
    Ok(connection)
}
