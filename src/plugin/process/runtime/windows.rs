// SPDX-License-Identifier: MPL-2.0

//! Event-driven helpers held in private, kill-on-close Windows jobs.
use super::{Start, Stream};
use crate::{
    plugin::{
        self,
        application::{Error, ErrorCode},
    },
    windows_process,
};
use std::{
    fs::OpenOptions,
    os::windows::io::OwnedHandle,
    path::{Path, PathBuf},
    process::{Command, ExitStatus},
    sync::{
        Arc, OnceLock,
        atomic::{AtomicBool, Ordering},
        mpsc as blocking,
    },
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::windows::named_pipe::NamedPipeServer,
    sync::{OwnedSemaphorePermit, Semaphore, mpsc, oneshot, watch},
    time::{Instant, sleep_until},
};

const GLOBAL_HELPERS: usize = 32;
const OUTPUT_CHUNK: usize = 16 * 1024;
const START_TIMEOUT: Duration = Duration::from_secs(5);
const WRITE_TIMEOUT: Duration = Duration::from_secs(5);
const FINAL_DRAIN: usize = 64 * 1024;

#[derive(Debug)]
pub struct Event {
    pub(crate) generation: String,
    pub(crate) process: String,
    pub(crate) kind: Kind,
    pub(crate) _permit: OwnedSemaphorePermit,
    pub(crate) _lifetime: Option<OwnedSemaphorePermit>,
}
#[derive(Debug)]
pub(crate) enum Kind {
    Started,
    StartFailed {
        error: Error,
    },
    Output {
        stream: Stream,
        bytes: Vec<u8>,
    },
    Written {
        request: String,
        result: Result<WriteResult, Error>,
    },
    CleanupDelayed,
    Exited {
        code: Option<i32>,
        signal: Option<i32>,
        error: Option<Error>,
        stdout: Vec<u8>,
        stderr: Vec<u8>,
        output_truncated: bool,
        write: Option<(String, Result<WriteResult, Error>)>,
    },
}
#[derive(Debug)]
pub(crate) struct WriteResult {
    pub written: usize,
    pub eof: bool,
}
#[derive(Debug)]
struct WriteCommand {
    request: String,
    bytes: Vec<u8>,
    eof: bool,
}
#[derive(Debug)]
pub(crate) struct Handle {
    writes: mpsc::Sender<WriteCommand>,
    stop: watch::Sender<bool>,
    writing: Arc<AtomicBool>,
}
impl Handle {
    pub fn write(&self, request: String, bytes: Vec<u8>, eof: bool) -> Result<(), Error> {
        if bytes.len() > super::MAX_IO_BYTES {
            return Err(error(
                ErrorCode::LimitExceeded,
                "Helper input exceeds its limit",
            ));
        }
        if *self.stop.borrow() {
            return Err(error(ErrorCode::Closed, "Helper is closing"));
        }
        if self.writing.swap(true, Ordering::AcqRel) {
            return Err(error(ErrorCode::Busy, "Helper already has a pending write"));
        }
        if self
            .writes
            .try_send(WriteCommand {
                request,
                bytes,
                eof,
            })
            .is_err()
        {
            self.writing.store(false, Ordering::Release);
            return Err(error(ErrorCode::Closed, "Helper input is closed"));
        }
        Ok(())
    }
    pub fn close(&self) {
        self.stop.send_replace(true);
    }
}
impl Drop for Handle {
    fn drop(&mut self) {
        self.close();
    }
}

fn error(code: ErrorCode, message: &str) -> Error {
    Error::new(code, message)
}

// At most 32 guards exist, each with a global permit. Their nonblocking Drop
// can therefore always enqueue into this 32-slot fallback. Its single thread
// is created only on first use and sleeps on recv, never on a polling timer.
struct Cleanup {
    child: windows_process::Child,
    lifetime: OwnedSemaphorePermit,
    completion: Option<Completion>,
    #[cfg(test)]
    reaped: Option<Arc<AtomicBool>>,
}
// A failed synchronous cleanup can be left in a spawn_blocking result when
// Tokio cancels its caller. This owner queues the child even if that result
// is dropped before the supervisor gets to handle it.
struct PendingCleanup {
    item: Option<Cleanup>,
    reaper: blocking::SyncSender<Cleanup>,
}
impl PendingCleanup {
    fn enqueue(mut self) {
        self.send();
    }
    fn send(&mut self) {
        let Some(item) = self.item.take() else { return };
        // Each item holds one of 32 global permits, so this item proves that
        // at most 31 other items can occupy the 32-slot channel.
        self.reaper
            .try_send(item)
            .unwrap_or_else(|_| unreachable!("reserved helper cleanup capacity"));
    }
}
impl Drop for PendingCleanup {
    fn drop(&mut self) {
        self.send();
    }
}
struct Completion {
    context: Context,
    runtime: tokio::runtime::Handle,
    notice: oneshot::Receiver<()>,
    failure: Option<Error>,
    tail: Tail,
    write: Option<(String, Result<WriteResult, Error>)>,
}
fn reaper() -> Result<blocking::SyncSender<Cleanup>, Error> {
    static REAPER: OnceLock<Result<blocking::SyncSender<Cleanup>, ()>> = OnceLock::new();
    REAPER
        .get_or_init(|| {
            let (sender, receiver) = blocking::sync_channel::<Cleanup>(GLOBAL_HELPERS);
            std::thread::Builder::new()
                .name("plugin-helper-reap".into())
                .spawn(move || {
                    let mut pending = std::collections::VecDeque::new();
                    loop {
                        if pending.is_empty() {
                            let Ok(cleanup) = receiver.recv() else { break };
                            pending.push_back(cleanup);
                        }
                        while let Ok(cleanup) = receiver.try_recv() {
                            pending.push_back(cleanup);
                        }
                        let mut cleanup = pending.pop_front().expect("pending cleanup");
                        match cleanup.child.terminate_and_wait_tree() {
                            Ok(status) => {
                                #[cfg(test)]
                                if let Some(reaped) = cleanup.reaped.take() {
                                    reaped.store(true, Ordering::Release);
                                }
                                if let Some(completion) = cleanup.completion.take() {
                                    let Completion {
                                        context,
                                        runtime,
                                        notice,
                                        failure,
                                        tail,
                                        write,
                                    } = completion;
                                    let lifetime = cleanup.lifetime;
                                    runtime.spawn(async move {
                                        // The delayed-cleanup notice is sent first. If its
                                        // supervisor is cancelled, a dropped sender releases
                                        // this wait and cleanup ownership is still settled.
                                        let _ = notice.await;
                                        context
                                            .final_output(
                                                Some(status),
                                                failure,
                                                lifetime,
                                                tail,
                                                write,
                                            )
                                            .await;
                                    });
                                }
                            }
                            Err(_) => {
                                pending.push_back(cleanup);
                                std::thread::sleep(Duration::from_millis(50));
                            }
                        }
                    }
                })
                .map(|_| sender)
                .map_err(|_| ())
        })
        .clone()
        .map_err(|_| error(ErrorCode::Unavailable, "Helper cleanup is unavailable"))
}
struct Guard {
    child: Option<windows_process::Child>,
    pipes: Option<Pipes>,
    lifetime: Option<OwnedSemaphorePermit>,
    reaper: blocking::SyncSender<Cleanup>,
    #[cfg(test)]
    fail_reap_once: bool,
    #[cfg(test)]
    reap_gate: Option<(blocking::Sender<()>, blocking::Receiver<()>)>,
    #[cfg(test)]
    reaped: Option<Arc<AtomicBool>>,
}
struct Pipes {
    stdin: NamedPipeServer,
    stdout: NamedPipeServer,
    stderr: Option<NamedPipeServer>,
}
impl Guard {
    fn kill(&mut self) {
        if let Some(child) = &mut self.child {
            let _ = child.kill();
        }
    }
    fn reap(mut self) -> Result<(Option<ExitStatus>, OwnedSemaphorePermit), Box<PendingCleanup>> {
        self.kill();
        self.pipes.take();
        let lifetime = self.lifetime.take().expect("helper lifetime");
        let Some(mut child) = self.child.take() else {
            return Ok((None, lifetime));
        };
        #[cfg(test)]
        if self.fail_reap_once {
            let pending = PendingCleanup {
                item: Some(Cleanup {
                    child,
                    lifetime,
                    completion: None,
                    reaped: self.reaped.take(),
                }),
                reaper: self.reaper.clone(),
            };
            if let Some((ready, release)) = self.reap_gate.take() {
                let _ = ready.send(());
                let _ = release.recv();
            }
            return Err(Box::new(pending));
        }
        match child.terminate_and_wait_tree() {
            Ok(status) => Ok((Some(status), lifetime)),
            Err(_) => Err(Box::new(PendingCleanup {
                item: Some(Cleanup {
                    child,
                    lifetime,
                    completion: None,
                    #[cfg(test)]
                    reaped: self.reaped.take(),
                }),
                reaper: self.reaper.clone(),
            })),
        }
    }
}
impl Drop for Guard {
    fn drop(&mut self) {
        self.kill();
        self.pipes.take();
        if let Some(child) = self.child.take() {
            let cleanup = Cleanup {
                child,
                lifetime: self.lifetime.take().expect("helper lifetime"),
                completion: None,
                #[cfg(test)]
                reaped: self.reaped.take(),
            };
            // Every queued item owns one of the 32 lifetime permits; this
            // additional item proves at most 31 can already be queued.
            self.reaper
                .try_send(cleanup)
                .unwrap_or_else(|_| unreachable!("reserved helper cleanup capacity"));
        }
    }
}

pub(crate) fn spawn(
    owner: usize,
    generation: String,
    process: String,
    root: PathBuf,
    start: Start,
    events: mpsc::Sender<plugin::Event>,
) -> Result<Handle, Error> {
    start.validate()?;
    spawn_with_launcher(
        owner,
        generation,
        process,
        events,
        Box::new(move |guard| spawn_child(root, start, guard)),
    )
}

type Launcher = Box<dyn FnOnce(&mut Guard) -> Result<(), Error> + Send>;
fn spawn_with_launcher(
    owner: usize,
    generation: String,
    process: String,
    events: mpsc::Sender<plugin::Event>,
    launcher: Launcher,
) -> Result<Handle, Error> {
    let runtime = tokio::runtime::Handle::try_current()
        .map_err(|_| error(ErrorCode::Unavailable, "Helper runtime is unavailable"))?;
    static HELPERS: OnceLock<Arc<Semaphore>> = OnceLock::new();
    let lifetime = HELPERS
        .get_or_init(|| Arc::new(Semaphore::new(GLOBAL_HELPERS)))
        .clone()
        .try_acquire_owned()
        .map_err(|_| error(ErrorCode::LimitExceeded, "Managed helper limit reached"))?;
    let reaper = reaper()?;
    let (writes, receiver) = mpsc::channel(1);
    let (stop, stopped) = watch::channel(false);
    let writing = Arc::new(AtomicBool::new(false));
    let context = Context {
        owner,
        generation,
        process,
        events,
        ordinary: Arc::new(Semaphore::new(3)),
        terminal: Arc::new(Semaphore::new(1)),
        stopped,
        writing: writing.clone(),
    };
    runtime.spawn(run(context, launcher, receiver, lifetime, reaper));
    Ok(Handle {
        writes,
        stop,
        writing,
    })
}

#[derive(Clone)]
struct Context {
    owner: usize,
    generation: String,
    process: String,
    events: mpsc::Sender<plugin::Event>,
    ordinary: Arc<Semaphore>,
    terminal: Arc<Semaphore>,
    stopped: watch::Receiver<bool>,
    writing: Arc<AtomicBool>,
}
impl Context {
    async fn emit(
        &self,
        kind: Kind,
        permit: OwnedSemaphorePermit,
        lifetime: Option<OwnedSemaphorePermit>,
    ) -> bool {
        let terminal = matches!(&kind, Kind::Exited { .. } | Kind::CleanupDelayed);
        let mut stopped = self.stopped.clone();
        if !terminal && *stopped.borrow() {
            return false;
        }
        let event = plugin::Event {
            plugin: self.owner,
            result: Ok(plugin::ClientMessage::Process(Event {
                generation: self.generation.clone(),
                process: self.process.clone(),
                kind,
                _permit: permit,
                _lifetime: lifetime,
            })),
        };
        if terminal {
            return self.events.send(event).await.is_ok();
        }
        tokio::select! {
            result = self.events.send(event) => result.is_ok(),
            _ = stopped.changed() => false,
        }
    }
    async fn ordinary(&self, kind: Kind) -> bool {
        let mut stopped = self.stopped.clone();
        if *stopped.borrow() {
            return false;
        }
        tokio::select! {
            permit = self.ordinary.clone().acquire_owned() => self.emit(kind, permit.expect("open helper permits"), None).await,
            _ = self.events.closed() => false,
            _ = stopped.changed() => false,
        }
    }
    async fn cleanup_delayed(&self) {
        let permit = self
            .terminal
            .clone()
            .acquire_owned()
            .await
            .expect("reserved terminal permit");
        self.emit(Kind::CleanupDelayed, permit, None).await;
    }
    async fn final_output(
        &self,
        status: Option<ExitStatus>,
        failure: Option<Error>,
        lifetime: OwnedSemaphorePermit,
        tail: Tail,
        write: Option<(String, Result<WriteResult, Error>)>,
    ) {
        let permit = self
            .terminal
            .clone()
            .acquire_owned()
            .await
            .expect("reserved final permit");
        let output_truncated = tail.truncated || failure.is_some();
        self.emit(
            Kind::Exited {
                code: status.and_then(|s| s.code()),
                signal: None,
                error: failure,
                stdout: tail.stdout,
                stderr: tail.stderr,
                output_truncated,
                write,
            },
            permit,
            Some(lifetime),
        )
        .await;
    }
}

async fn finish(
    context: Context,
    result: Result<(Option<ExitStatus>, OwnedSemaphorePermit), Box<PendingCleanup>>,
    failure: Option<Error>,
    tail: Tail,
    write: Option<(String, Result<WriteResult, Error>)>,
) {
    match result {
        Ok((status, lifetime)) => {
            context
                .final_output(status, failure, lifetime, tail, write)
                .await;
        }
        Err(mut cleanup) => {
            let (notice, sent) = oneshot::channel();
            let mut tail = tail;
            tail.truncated = true;
            cleanup.item.as_mut().expect("pending cleanup").completion = Some(Completion {
                context: context.clone(),
                runtime: tokio::runtime::Handle::current(),
                notice: sent,
                failure,
                tail,
                write,
            });
            cleanup.enqueue();
            context.cleanup_delayed().await;
            let _ = notice.send(());
        }
    }
}

fn spawn_child(root: PathBuf, start: Start, guard: &mut Guard) -> Result<(), Error> {
    let root = match root.canonicalize() {
        Ok(root) => root,
        Err(_) => {
            return Err(error(
                ErrorCode::NotFound,
                "Helper workspace is unavailable",
            ));
        }
    };
    let candidate = start
        .cwd
        .as_ref()
        .map_or_else(|| root.clone(), |cwd| root.join(cwd));
    let cwd = match candidate.canonicalize() {
        Ok(cwd) if cwd.starts_with(&root) && cwd.is_dir() => cwd,
        _ => {
            return Err(error(
                ErrorCode::InvalidArgument,
                "Helper directory is outside the workspace or unavailable",
            ));
        }
    };
    let executable = crate::service_health::resolve_configured_executable(
        Path::new(&start.executable),
        std::env::var_os("PATH").as_deref(),
    )
    .ok_or_else(|| error(ErrorCode::Unavailable, "Helper executable is unavailable"))?;
    let (stdin, child_stdin) = windows_process::overlapped::pipe(false)
        .map_err(|_| error(ErrorCode::Unavailable, "Helper pipes are unavailable"))?;
    let (stdout, child_stdout) = windows_process::overlapped::pipe(true)
        .map_err(|_| error(ErrorCode::Unavailable, "Helper pipes are unavailable"))?;
    let (stderr, child_stderr): (Option<NamedPipeServer>, OwnedHandle) = if start.capture_stderr {
        let (parent, child) = windows_process::overlapped::pipe(true)
            .map_err(|_| error(ErrorCode::Unavailable, "Helper pipes are unavailable"))?;
        (Some(parent), child)
    } else {
        let sink = OpenOptions::new()
            .write(true)
            .open("NUL")
            .map_err(|_| error(ErrorCode::Unavailable, "Helper error sink is unavailable"))?;
        (None, sink.into())
    };
    let mut command = Command::new(executable);
    command.args(start.args).current_dir(cwd);
    let child =
        windows_process::spawn_with_stdio(&command, [child_stdin, child_stdout, child_stderr])
            .map_err(|_| error(ErrorCode::Unavailable, "Helper could not be started"))?;
    guard.child = Some(child);
    guard.pipes = Some(Pipes {
        stdin,
        stdout,
        stderr,
    });
    Ok(())
}

async fn run(
    mut context: Context,
    launcher: Launcher,
    mut commands: mpsc::Receiver<WriteCommand>,
    lifetime: OwnedSemaphorePermit,
    reaper: blocking::SyncSender<Cleanup>,
) {
    let guard = Guard {
        child: None,
        pipes: None,
        lifetime: Some(lifetime),
        reaper: reaper.clone(),
        #[cfg(test)]
        fail_reap_once: false,
        #[cfg(test)]
        reap_gate: None,
        #[cfg(test)]
        reaped: None,
    };
    let mut spawning = tokio::task::spawn_blocking(move || {
        let mut guard = guard;
        let result =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| launcher(&mut guard)))
                .unwrap_or_else(|_| Err(error(ErrorCode::Internal, "Helper startup failed")));
        (guard, result)
    });
    let mut cancelled_start = *context.stopped.borrow();
    let result = if cancelled_start {
        None
    } else {
        tokio::select! {
            result = &mut spawning => Some(result),
            _ = tokio::time::sleep(START_TIMEOUT) => {
                context.ordinary(Kind::StartFailed { error: error(ErrorCode::Unavailable, "Helper startup timed out") }).await;
                cancelled_start = true;
                None
            },
            _ = context.stopped.changed() => { cancelled_start = true; None },
            _ = context.events.closed() => { cancelled_start = true; None },
        }
    };
    // Never detach a late spawn: its guarded result is still consumed and
    // cleaned up. Shutdown drops the result into the same bounded reaper.
    let (mut guard, spawned) = match result {
        Some(result) => result,
        None => spawning.await,
    }
    .expect("non-aborted helper spawn");
    if cancelled_start || *context.stopped.borrow() {
        let result = tokio::task::spawn_blocking(move || guard.reap())
            .await
            .expect("helper reap");
        finish(
            context,
            result,
            Some(error(ErrorCode::Cancelled, "Helper startup was cancelled")),
            Tail::default(),
            None,
        )
        .await;
        return;
    }
    if let Err(failure) = spawned {
        let result = tokio::task::spawn_blocking(move || guard.reap())
            .await
            .expect("helper reap");
        finish(context, result, Some(failure), Tail::default(), None).await;
        return;
    }
    let observer = match guard.child.as_ref().expect("spawned child").exit_watcher() {
        Ok(observer) => observer,
        Err(_) => {
            let result = tokio::task::spawn_blocking(move || guard.reap())
                .await
                .expect("helper reap");
            finish(
                context,
                result,
                Some(error(
                    ErrorCode::Unavailable,
                    "Helper exit notifications are unavailable",
                )),
                Tail::default(),
                None,
            )
            .await;
            return;
        }
    };
    let Pipes {
        stdin,
        stdout,
        stderr,
    } = guard.pipes.take().expect("spawned pipes");
    let mut stdin = Some(stdin);
    let mut stdout = Some(stdout);
    let mut stderr = stderr;
    let mut pending: Option<(WriteCommand, usize, Instant)> = None;
    let mut failure = None;
    if !*context.stopped.borrow() {
        context.ordinary(Kind::Started).await;
    }
    let mut permit = None;
    let mut out = [0u8; OUTPUT_CHUNK];
    let mut err = [0u8; OUTPUT_CHUNK];
    loop {
        if *context.stopped.borrow() || context.events.is_closed() {
            break;
        }
        if let Some((command, written, _)) = &pending {
            if *written == command.bytes.len() {
                if command.eof {
                    stdin.take();
                }
                if let Some(permit) = permit.take() {
                    let (command, written, _) = pending.take().expect("completed write");
                    context.writing.store(false, Ordering::Release);
                    if !context
                        .emit(
                            Kind::Written {
                                request: command.request,
                                result: Ok(WriteResult {
                                    written,
                                    eof: command.eof,
                                }),
                            },
                            permit,
                            None,
                        )
                        .await
                    {
                        break;
                    }
                    continue;
                }
            } else if stdin.is_none() {
                failure = Some(error(ErrorCode::Closed, "Helper input is closed"));
                break;
            }
        }
        let deadline = pending
            .as_ref()
            .map(|(_, _, deadline)| *deadline)
            .unwrap_or_else(|| Instant::now() + WRITE_TIMEOUT);
        tokio::select! {
            _ = context.stopped.changed() => {},
            _ = context.events.closed() => break,
            completed = observer.wait() => {
                if completed.is_err() {
                    failure = Some(error(ErrorCode::Unavailable, "Helper exit state is unavailable"));
                }
                break;
            },
            _ = sleep_until(deadline), if pending.as_ref().is_some_and(|(command, written, _)| *written < command.bytes.len()) => { failure = Some(error(ErrorCode::OutcomeUnknown, "Helper input timed out; delivery may be partial")); break; },
            command = commands.recv(), if pending.is_none() => match command {
                Some(command) => pending = Some((command, 0, Instant::now() + WRITE_TIMEOUT)),
                None => break,
            },
            acquired = context.ordinary.clone().acquire_owned(), if permit.is_none() => { permit = acquired.ok(); },
            result = async { stdout.as_mut().expect("stdout").read(&mut out).await }, if stdout.is_some() && permit.is_some() => match result {
                Ok(0) => { stdout.take(); },
                Err(ref error) if error.kind() == std::io::ErrorKind::BrokenPipe => { stdout.take(); },
                Ok(count) => { if !context.emit(Kind::Output { stream: Stream::Stdout, bytes: out[..count].to_vec() }, permit.take().expect("output permit"), None).await { break; } },
                Err(_) => { failure = Some(error(ErrorCode::Unavailable, "Helper output could not be read")); break; },
            },
            result = async { stderr.as_mut().expect("stderr").read(&mut err).await }, if stderr.is_some() && permit.is_some() => match result {
                Ok(0) => { stderr.take(); },
                Err(ref error) if error.kind() == std::io::ErrorKind::BrokenPipe => { stderr.take(); },
                Ok(count) => { if !context.emit(Kind::Output { stream: Stream::Stderr, bytes: err[..count].to_vec() }, permit.take().expect("output permit"), None).await { break; } },
                Err(_) => { failure = Some(error(ErrorCode::Unavailable, "Helper error output could not be read")); break; },
            },
            result = async { let (command, written, _) = pending.as_ref().expect("write"); stdin.as_mut().expect("stdin").write(&command.bytes[*written..]).await }, if pending.as_ref().is_some_and(|(command, written, _)| *written < command.bytes.len()) && stdin.is_some() => match result {
                Ok(0) | Err(_) => { failure = Some(error(ErrorCode::OutcomeUnknown, "Helper input failed; delivery may be partial")); break; },
                Ok(count) => { pending.as_mut().expect("write").1 += count; },
            },
        }
    }
    // Kill before waiting for queue capacity or reaping. No output flood can
    // postpone private job cleanup.
    guard.kill();
    drop(observer);
    drop(permit);
    stdin.take();
    let result = tokio::task::spawn_blocking(move || guard.reap())
        .await
        .expect("helper reap");
    let write = pending
        .take()
        .or_else(|| {
            commands
                .try_recv()
                .ok()
                .map(|command| (command, 0, Instant::now()))
        })
        .map(|(command, written, _)| {
            context.writing.store(false, Ordering::Release);
            let result = if written == command.bytes.len() {
                Ok(WriteResult {
                    written,
                    eof: command.eof,
                })
            } else {
                Err(error(
                    ErrorCode::OutcomeUnknown,
                    "Helper stopped; input delivery may be partial",
                ))
            };
            (command.request, result)
        });
    // Tail bytes travel in the reserved terminal event, so saturated ordinary
    // permits cannot block either reaping or final delivery.
    let mut remaining = FINAL_DRAIN;
    let deadline = Instant::now() + Duration::from_millis(80);
    let (tail_out, out_truncated) = drain(&mut stdout, &mut remaining, deadline).await;
    let (tail_err, err_truncated) = drain(&mut stderr, &mut remaining, deadline).await;
    drop((stdout, stderr));
    finish(
        context,
        result,
        failure,
        Tail {
            stdout: tail_out,
            stderr: tail_err,
            truncated: out_truncated || err_truncated,
        },
        write,
    )
    .await;
}

#[derive(Default)]
struct Tail {
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    truncated: bool,
}

async fn drain<R: tokio::io::AsyncRead + Unpin>(
    pipe: &mut Option<R>,
    remaining: &mut usize,
    deadline: Instant,
) -> (Vec<u8>, bool) {
    let Some(pipe) = pipe else {
        return (Vec::new(), false);
    };
    let mut tail = Vec::new();
    while *remaining > 0 {
        if Instant::now() >= deadline {
            return (tail, true);
        }
        let mut bytes = [0u8; OUTPUT_CHUNK];
        let limit = (*remaining).min(OUTPUT_CHUNK);
        let read = tokio::time::timeout_at(deadline, pipe.read(&mut bytes[..limit])).await;
        let count = match read {
            Ok(Ok(count)) => count,
            Ok(Err(error)) if error.kind() == std::io::ErrorKind::BrokenPipe => 0,
            _ => return (tail, true),
        };
        if count == 0 {
            return (tail, false);
        }
        tail.extend_from_slice(&bytes[..count]);
        *remaining -= count;
    }
    (tail, true)
}

#[cfg(test)]
#[path = "../../tests/process_runtime_windows.rs"]
mod tests;
