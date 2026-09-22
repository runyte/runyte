// SPDX-License-Identifier: MPL-2.0

//! Native plugin process ownership over overlapped private pipes.

use super::*;
use std::{fs::OpenOptions, os::windows::io::OwnedHandle, process::Command};

pub(super) async fn supervise(
    config: PluginConfig,
    root: PathBuf,
    plugin: usize,
    events: &mpsc::Sender<Event>,
    input: mpsc::Receiver<HostMessage>,
    admission: OutputAdmission,
    control: WorkerControl,
) -> (Option<&'static str>, bool) {
    let WorkerControl {
        mut cancellation,
        rejection,
    } = control;
    if *cancellation.borrow() {
        return (None, true);
    }

    let (stdin, child_stdin) = match crate::windows_process::overlapped::pipe(false) {
        Ok(pipe) => pipe,
        Err(_) => return (Some("Plugin process could not start"), true),
    };
    let (stdout, child_stdout) = match crate::windows_process::overlapped::pipe(true) {
        Ok(pipe) => pipe,
        Err(_) => return (Some("Plugin process could not start"), true),
    };
    let stderr: OwnedHandle = match OpenOptions::new().write(true).open("NUL") {
        Ok(file) => file.into(),
        Err(_) => return (Some("Plugin process could not start"), true),
    };
    let mut command = Command::new(&config.executable);
    command.args(&config.args).current_dir(&root);
    let child = match crate::windows_process::spawn_with_stdio(
        &command,
        [child_stdin, child_stdout, stderr],
    ) {
        Ok(child) => child,
        Err(_) => return (Some("Plugin process could not start"), true),
    };
    let exit = match child.exit_watcher() {
        Ok(exit) => exit,
        Err(_) => {
            let reaped = cleanup(child).await;
            return (Some("Plugin process could not start"), reaped);
        }
    };

    let mut writer = stdin;
    let write_uncertain = AtomicBool::new(false);
    let result = {
        let io = std::panic::AssertUnwindSafe(run(
            plugin,
            events,
            input,
            admission,
            RunTransport {
                reader: stdout,
                writer: &mut writer,
                child_exit: exit.wait(),
                drain_after_exit: true,
                write_uncertain: Some(&write_uncertain),
            },
        ))
        .catch_unwind();
        tokio::select! {
            biased;
            _ = async {
                while !*cancellation.borrow_and_update() {
                    if cancellation.changed().await.is_err() { break; }
                }
            } => Ok(Ok(())),
            result = io => result,
        }
    };

    if let Some(error) = writable_rejection(&rejection, &write_uncertain) {
        let message = application::HostMessage::RegistrationError { error };
        if let Ok(mut bytes) = serde_json::to_vec(&message) {
            bytes.push(b'\n');
            let _ = write_message(&mut writer, &bytes, Some(&write_uncertain)).await;
        }
    }
    drop(writer);
    drop(exit);
    let reaped = cleanup(child).await;
    if !reaped {
        return (Some("Plugin process cleanup failed"), false);
    }
    let failure = match result {
        Ok(Ok(())) => None,
        Ok(Err(error)) => Some(match error.to_string().as_str() {
            "plugin registration or invocation timed out" => "Plugin request timed out",
            "plugin stopped reading" => "Plugin stopped reading host messages",
            "plugin inbound queue full" | "plugin inbound byte quota exceeded" => {
                "Plugin output exceeded its quota"
            }
            reason if reason.starts_with("plugin exited:") => "Plugin process exited",
            _ => "Plugin protocol or IO failed",
        }),
        Err(_) => Some("Plugin worker failed"),
    };
    (failure, true)
}

fn writable_rejection(
    rejection: &std::sync::Mutex<Option<application::RegistrationFailure>>,
    write_uncertain: &AtomicBool,
) -> Option<application::RegistrationFailure> {
    let rejected = rejection.lock().expect("rejection lock").take();
    (!write_uncertain.load(Ordering::Acquire))
        .then_some(rejected)
        .flatten()
}

async fn cleanup(mut child: crate::windows_process::Child) -> bool {
    tokio::task::spawn_blocking(move || child.terminate_and_wait_tree().map(|_| ()))
        .await
        .is_ok_and(|result| result.is_ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io,
        pin::Pin,
        sync::atomic::Ordering,
        task::{Context, Poll},
    };

    struct PartialThenPending(bool);
    impl AsyncWrite for PartialThenPending {
        fn poll_write(
            mut self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
            bytes: &[u8],
        ) -> Poll<io::Result<usize>> {
            if self.0 {
                Poll::Pending
            } else {
                self.0 = true;
                Poll::Ready(Ok(bytes.len().min(1)))
            }
        }

        fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
            Poll::Ready(Ok(()))
        }

        fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
            Poll::Ready(Ok(()))
        }
    }

    #[tokio::test]
    async fn cancelled_partial_write_suppresses_registration_error_frame() {
        let uncertain = AtomicBool::new(false);
        let mut writer = PartialThenPending(false);
        {
            let mut writing = std::pin::pin!(write_message(
                &mut writer,
                b"partial frame\n",
                Some(&uncertain),
            ));
            tokio::select! {
                biased;
                _ = tokio::time::sleep(Duration::from_millis(10)) => {}
                result = &mut writing => panic!("partial writer unexpectedly settled: {result:?}"),
            }
        }
        assert!(uncertain.load(Ordering::Acquire));

        let rejection = std::sync::Mutex::new(Some(application::RegistrationFailure {
            code: "unsupported_feature",
            message: "Unavailable".into(),
        }));
        assert!(writable_rejection(&rejection, &uncertain).is_none());
        assert!(rejection.lock().unwrap().is_none());
    }
}
