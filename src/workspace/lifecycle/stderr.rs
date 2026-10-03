// SPDX-License-Identifier: MPL-2.0

//! Bounded startup diagnostics whose reader follows the direct host lifetime.

use std::{
    io::{self, Read},
    os::fd::AsRawFd,
    process::ChildStderr,
};
use tokio::{io::unix::AsyncFd, sync::oneshot, task::JoinHandle};

const RETAINED_BYTES: usize = 16 * 1024;
const READ_SIZE: usize = 8192;
const READS_PER_TURN: usize = 8;

pub(super) struct Capture {
    stop: Option<oneshot::Sender<()>>,
    task: Option<JoinHandle<Diagnostics>>,
}

impl Capture {
    pub(super) fn new(pipe: ChildStderr) -> io::Result<Self> {
        let descriptor = pipe.as_raw_fd();
        // SAFETY: the owned pipe keeps this descriptor live through both
        // fcntl calls. Only the reader's file status flags are changed.
        let flags = unsafe { libc::fcntl(descriptor, libc::F_GETFL) };
        if flags == -1
            || unsafe { libc::fcntl(descriptor, libc::F_SETFL, flags | libc::O_NONBLOCK) } == -1
        {
            return Err(io::Error::last_os_error());
        }
        let reader = AsyncFd::new(pipe)?;
        let (stop, receiver) = oneshot::channel();
        Ok(Self {
            stop: Some(stop),
            task: Some(tokio::spawn(drain(reader, receiver))),
        })
    }

    pub(super) async fn finish(mut self) -> String {
        self.stop();
        self.task
            .take()
            .expect("capture task retained")
            .await
            .map(|diagnostics| diagnostics.text())
            .unwrap_or_default()
    }

    fn stop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
    }
}

impl Drop for Capture {
    fn drop(&mut self) {
        self.stop();
    }
}

#[derive(Default)]
struct Diagnostics {
    bytes: Vec<u8>,
    truncated: bool,
}

impl Diagnostics {
    fn retain(&mut self, bytes: &[u8]) {
        let retained = bytes.len().min(RETAINED_BYTES - self.bytes.len());
        self.bytes.extend_from_slice(&bytes[..retained]);
        self.truncated |= retained < bytes.len();
    }

    fn text(self) -> String {
        let mut text = String::from_utf8_lossy(&self.bytes).trim().to_owned();
        if self.truncated {
            text.push_str(" [startup stderr truncated]");
        }
        text
    }
}

async fn drain(mut reader: AsyncFd<ChildStderr>, mut stop: oneshot::Receiver<()>) -> Diagnostics {
    let mut diagnostics = Diagnostics::default();
    let mut bytes = [0; READ_SIZE];
    loop {
        tokio::select! {
            biased;
            _ = &mut stop => break,
            ready = reader.readable_mut() => {
                let Ok(mut ready) = ready else { return diagnostics };
                for _ in 0..READS_PER_TURN {
                    match ready.try_io(|reader| reader.get_mut().read(&mut bytes)) {
                        Ok(Ok(0)) => return diagnostics,
                        Ok(Ok(count)) => diagnostics.retain(&bytes[..count]),
                        Ok(Err(error)) if error.kind() == io::ErrorKind::Interrupted => {},
                        Ok(Err(_)) => return diagnostics,
                        Err(_) => break,
                    }
                }
            }
        }
        // A continuously noisy child cannot monopolize its caller's runtime.
        tokio::task::yield_now().await;
    }
    // Capture bytes already queued at direct-child exit. A descendant may
    // retain or continuously write the pipe: neither EOF nor unlimited drain
    // work is required to finish the original child's failure report.
    for _ in 0..READS_PER_TURN {
        match reader.get_mut().read(&mut bytes) {
            Ok(0) => break,
            Ok(count) => diagnostics.retain(&bytes[..count]),
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(_) => break,
        }
    }
    diagnostics
}
