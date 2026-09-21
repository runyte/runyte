// SPDX-License-Identifier: MPL-2.0

//! Overlapped stdio and job ownership; no runtime blocking-pool pipe workers.
use super::*;
use std::{
    future::Future,
    io,
    os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle},
    process::Command,
    ptr,
    task::{Context, Poll, Waker},
};
use tokio::{
    net::windows::named_pipe::{NamedPipeServer, ServerOptions},
    task::AbortHandle,
};
use windows_sys::Win32::{
    Foundation::{GENERIC_READ, GENERIC_WRITE, INVALID_HANDLE_VALUE},
    Security::Cryptography::{BCRYPT_USE_SYSTEM_PREFERRED_RNG, BCryptGenRandom},
    Storage::FileSystem::{CreateFileW, FILE_ATTRIBUTE_NORMAL, OPEN_EXISTING},
    System::{
        Pipes::{GetNamedPipeClientProcessId, GetNamedPipeServerProcessId},
        Threading::GetCurrentProcessId,
    },
};

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

fn pipe(parent_reads: bool) -> io::Result<(NamedPipeServer, OwnedHandle)> {
    let mut nonce = [0u8; 16];
    let status = unsafe {
        BCryptGenRandom(
            ptr::null_mut(),
            nonce.as_mut_ptr(),
            nonce.len() as u32,
            BCRYPT_USE_SYSTEM_PREFERRED_RNG,
        )
    };
    if status < 0 {
        return Err(io::Error::other(
            "cannot allocate a private language-server pipe name",
        ));
    }
    let nonce = nonce
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let name = format!(r"\\.\pipe\runyte-lsp-{}-{nonce}", std::process::id());
    let server = crate::windows_fs::with_private_security(|security| unsafe {
        ServerOptions::new()
            .access_inbound(parent_reads)
            .access_outbound(!parent_reads)
            .first_pipe_instance(true)
            .max_instances(1)
            .reject_remote_clients(true)
            .in_buffer_size(64 * 1024)
            .out_buffer_size(64 * 1024)
            .create_with_security_attributes_raw(
                &name,
                (security as *const windows_sys::Win32::Security::SECURITY_ATTRIBUTES)
                    .cast_mut()
                    .cast(),
            )
    })?;
    let name: Vec<_> = name.encode_utf16().chain(Some(0)).collect();
    // Only the child endpoint is synchronous. It stays non-inheritable here;
    // windows_process duplicates it into its isolated inheritance surrogate.
    let client = unsafe {
        CreateFileW(
            name.as_ptr(),
            if parent_reads {
                GENERIC_WRITE
            } else {
                GENERIC_READ
            },
            0,
            ptr::null(),
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            ptr::null_mut(),
        )
    };
    if client == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    let client = unsafe { OwnedHandle::from_raw_handle(client) };
    // Our synchronous client open has already connected. Tokio owns any
    // overlapped connect state, including cancellation on the failure path.
    let connected = {
        let mut connect = std::pin::pin!(server.connect());
        connect
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
    };
    match connected {
        Poll::Ready(result) => result?,
        Poll::Pending => {
            return Err(io::Error::other(
                "private language-server pipe did not connect",
            ));
        }
    }
    let mut client_pid = 0;
    let mut server_pid = 0;
    if unsafe { GetNamedPipeClientProcessId(server.as_raw_handle(), &mut client_pid) } == 0
        || unsafe { GetNamedPipeServerProcessId(client.as_raw_handle(), &mut server_pid) } == 0
    {
        return Err(io::Error::last_os_error());
    }
    let own_pid = unsafe { GetCurrentProcessId() };
    if client_pid != own_pid || server_pid != own_pid {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "private language-server pipe peer is not the launching process",
        ));
    }
    Ok((server, client))
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
