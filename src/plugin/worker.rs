// SPDX-License-Identifier: MPL-2.0

use super::*;
use anyhow::{Context, Result, bail, ensure};
use futures_util::FutureExt;
#[cfg(not(windows))]
use std::process::Stdio;
use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};
use std::{path::PathBuf, time::Duration};
use tokio::{
    io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader},
    sync::{Notify, mpsc, watch},
};

#[cfg(windows)]
mod native;

const PRODUCER_EVENTS: usize = 16;
const RESERVED_MESSAGES: usize = 8;
const RESERVED_BYTES: usize = 512 * 1024;
/// Each worker owns 16 input/deadline permits, one state-ready notice and one
/// final settled event. Local IO retains its separate 16 permits until its result is read.
/// Each of 32 managed helpers reserves three ordinary events and one final reap.
pub const EVENT_CAPACITY: usize = MAX_PLUGINS * (PRODUCER_EVENTS + 2) + 16 + 32 * 4;

#[derive(Debug)]
pub struct Event {
    pub plugin: usize,
    pub result: Result<ClientMessage, String>,
}

/// Dropping an owner requests cancellation. The detached supervisor retains the
/// child until reaping and its final event until the host consumes it.
pub struct Worker {
    cancelled: watch::Sender<bool>,
    settled: tokio::sync::oneshot::Receiver<bool>,
    rejection: Arc<std::sync::Mutex<Option<application::RegistrationFailure>>>,
}
impl Worker {
    /// The supervisor writes this small final frame after dropping ordinary IO,
    /// with the existing two-second write bound, then kills and reaps the child.
    pub(crate) fn reject(&self, error: application::RegistrationFailure) {
        *self.rejection.lock().expect("rejection lock") = Some(error);
        self.stop();
    }

    pub fn stop(&self) {
        self.cancelled.send_replace(true);
    }

    /// Shutdown must retain the runtime until the child has been reaped, even
    /// when the host has stopped draining the ordinary event queue.
    pub(crate) async fn wait_stopped(mut self) -> Result<()> {
        self.stop();
        ensure!(
            (&mut self.settled)
                .await
                .context("Plugin cleanup task stopped")?,
            "Plugin process cleanup failed"
        );
        Ok(())
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.stop();
    }
}

pub fn spawn(
    config: PluginConfig,
    root: PathBuf,
    plugin: usize,
    events: mpsc::Sender<Event>,
) -> (Worker, Sender) {
    let (sender, receiver) = mpsc::channel(32);
    let sender = Sender::bounded(sender, application::MAX_QUEUE_BYTES);
    let admission = OutputAdmission {
        charge: Arc::clone(&sender.bytes),
        output: Arc::clone(&sender.output),
        limit: sender.limit,
    };
    let (cancelled, cancellation) = watch::channel(false);
    let (settled, completion) = tokio::sync::oneshot::channel();
    let rejection = Arc::new(std::sync::Mutex::new(None));
    let worker_rejection = Arc::clone(&rejection);
    tokio::spawn(async move {
        let (failure, reaped) = supervise(
            config,
            root,
            plugin,
            &events,
            receiver,
            admission,
            WorkerControl {
                cancellation,
                rejection: worker_rejection,
            },
        )
        .await;
        let _ = settled.send(reaped);
        // This is the worker's only terminal/failure event. It follows every
        // input, deadline and readiness notice from this same task in FIFO order.
        let _ = events
            .send(Event {
                plugin,
                result: Ok(ClientMessage::WorkerStopped {
                    failure: failure.map(str::to_owned),
                    reaped,
                }),
            })
            .await;
    });
    (
        Worker {
            cancelled,
            settled: completion,
            rejection,
        },
        sender,
    )
}

async fn read_message<R: AsyncRead + Unpin>(
    reader: &mut BufReader<R>,
) -> Result<Option<(ClientMessage, usize)>> {
    let mut bytes = Vec::new();
    let n = match reader
        .take((MAX_BYTES + 1) as u64)
        .read_until(b'\n', &mut bytes)
        .await
    {
        Ok(n) => n,
        #[cfg(windows)]
        Err(error) if error.kind() == std::io::ErrorKind::BrokenPipe && bytes.is_empty() => {
            return Ok(None);
        }
        #[cfg(windows)]
        Err(error) if error.kind() == std::io::ErrorKind::BrokenPipe => bytes.len(),
        Err(error) => return Err(error.into()),
    };
    if n == 0 {
        return Ok(None);
    }
    ensure!(
        n <= MAX_BYTES && bytes.last() == Some(&b'\n'),
        "plugin message exceeds limit or lacks newline"
    );
    application::decode(&bytes)
        .map(|message| Some((message, n)))
        .context("invalid application message")
}

struct OutputAdmission {
    charge: Arc<AtomicUsize>,
    output: Arc<OutputWake>,
    limit: usize,
}

struct WorkerControl {
    cancellation: watch::Receiver<bool>,
    rejection: Arc<std::sync::Mutex<Option<application::RegistrationFailure>>>,
}

#[cfg(windows)]
async fn supervise(
    config: PluginConfig,
    root: PathBuf,
    plugin: usize,
    events: &mpsc::Sender<Event>,
    input: mpsc::Receiver<HostMessage>,
    admission: OutputAdmission,
    control: WorkerControl,
) -> (Option<&'static str>, bool) {
    native::supervise(config, root, plugin, events, input, admission, control).await
}

#[cfg(not(windows))]
async fn supervise(
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
    let mut child = match tokio::process::Command::new(&config.executable)
        .args(&config.args)
        .current_dir(root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
    {
        Ok(child) => child,
        Err(_) => return (Some("Plugin process could not start"), true),
    };
    let mut writer = child.stdin.take().expect("piped stdin");
    let result = {
        let io = std::panic::AssertUnwindSafe(run(
            plugin,
            events,
            input,
            admission,
            RunTransport {
                reader: child.stdout.take().expect("piped stdout"),
                writer: &mut writer,
                child_exit: child.wait(),
                drain_after_exit: false,
                write_uncertain: None,
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
    let rejected = rejection.lock().expect("rejection lock").take();
    if let Some(error) = rejected {
        let message = application::HostMessage::RegistrationError { error };
        if let Ok(mut bytes) = serde_json::to_vec(&message) {
            bytes.push(b'\n');
            let _ = tokio::time::timeout(Duration::from_secs(2), writer.write_all(&bytes)).await;
        }
    }
    // Cancellation also interrupts a blocked stdin write. No new producer event
    // can appear after dropping the IO future, and cleanup never blocks input.
    let _ = child.start_kill();
    let reaped = child.wait().await;
    if reaped.is_err() {
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

struct RunTransport<'a, R, W, E> {
    reader: R,
    writer: &'a mut W,
    child_exit: E,
    drain_after_exit: bool,
    write_uncertain: Option<&'a AtomicBool>,
}

async fn run<R, W, E>(
    plugin: usize,
    events: &mpsc::Sender<Event>,
    mut input: mpsc::Receiver<HostMessage>,
    admission: OutputAdmission,
    transport: RunTransport<'_, R, W, E>,
) -> Result<()>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
    E: std::future::Future<Output = std::io::Result<std::process::ExitStatus>>,
{
    let RunTransport {
        reader,
        writer,
        child_exit,
        drain_after_exit,
        write_uncertain,
    } = transport;
    let OutputAdmission {
        charge,
        output,
        limit,
    } = admission;
    let mut reader = BufReader::new(reader);
    let mut child_exit = std::pin::pin!(child_exit);
    let mut exited = None;
    let mut exit_drain = None;
    let mut write_failure = None;
    let mut deadline = Some(tokio::time::Instant::now() + TIMEOUT);
    let slots = Arc::new(tokio::sync::Semaphore::new(PRODUCER_EVENTS));
    let inbound_bytes = Arc::new(tokio::sync::Semaphore::new(application::MAX_QUEUE_BYTES));
    let mut deadlines = BTreeMap::<String, tokio::time::Instant>::new();
    // Keep the partially read message future across outbound messages: cancelling
    // read_until would lose bytes already consumed from the pipe.
    loop {
        let mut reading = Box::pin(read_message(&mut reader));
        loop {
            tokio::select! {
                result = &mut reading => {
                    let Some((message, size)) = result? else {
                        if let Some(status) = exited {
                            if let Some(error) = write_failure.take() {
                                return Err(error);
                            }
                            bail!("plugin exited: {status}");
                        }
                        if drain_after_exit {
                            let drain = exit_drain.unwrap_or_else(|| {
                                tokio::time::Instant::now() + Duration::from_secs(2)
                            });
                            let status = tokio::time::timeout_at(drain, &mut child_exit)
                            .await
                            .context("plugin exit was not observed after stdout closed")??;
                            if let Some(error) = write_failure.take() {
                                return Err(error);
                            }
                            bail!("plugin exited: {status}");
                        }
                        bail!("plugin closed stdout");
                    };
                    // Internal deadlines share producer admission and
                    // consume these same permits rather than another owner's
                    // reserved space in the host channel.
                    let message = queued(message, size, &slots, &inbound_bytes)?;
                    events.try_send(Event { plugin, result: Ok(message) })
                        .map_err(|_| anyhow::anyhow!("plugin inbound queue full or closed"))?;
                    break;
                }
                message = input.recv(), if exited.is_none() && write_failure.is_none() => {
                    let Some(message) = message else { return Ok(()); };
                    charge.fetch_sub(encoded_len(&message)?, Ordering::Relaxed);
                    if let HostMessage::Deadline { token, after_ms } = message {
                        if let Some(ms) = after_ms { deadlines.insert(token, tokio::time::Instant::now() + Duration::from_millis(ms)); }
                        else { deadlines.remove(&token); }
                        continue;
                    }
                    if let HostMessage::Application(application::HostMessage::Registered { .. }) = &message { deadline = None; }
                    let mut bytes = serde_json::to_vec(&message)?;
                    ensure!(bytes.len() < MAX_BYTES, "plugin outbound message exceeds limit");
                    bytes.push(b'\n');
                    if let Err(error) = write_message(writer, &bytes, write_uncertain).await {
                        if !drain_after_exit {
                            return Err(error);
                        }
                        write_failure = Some(error);
                        deadline = None;
                        deadlines.clear();
                        exit_drain = Some(tokio::time::Instant::now() + Duration::from_secs(2));
                    }
                }
                _ = output.changed.notified(), if exited.is_none() && write_failure.is_none() => {}
                permit = events.reserve(), if exited.is_none() && write_failure.is_none() && output.ready(input.capacity(), charge.load(Ordering::Acquire), limit) => {
                    let permit = permit.context("plugin event channel closed")?;
                    let notification = output.notification();
                    permit.send(Event { plugin, result: Ok(ClientMessage::OutputReady { _notification: notification }) });
                }
                _ = async { match deadline {
                    Some(deadline) => tokio::time::sleep_until(deadline).await,
                    None => std::future::pending().await,
                }}, if exited.is_none() && write_failure.is_none() => bail!("plugin registration or invocation timed out"),
                permit = async {
                    match deadlines.values().min().copied() {
                        Some(at) => tokio::time::sleep_until(at).await,
                        None => std::future::pending().await,
                    }
                    // Host timers can expire together without an overloaded
                    // plugin. Wait for this owner's admission inside select so
                    // pipe IO and other owners remain responsive meanwhile.
                    Arc::clone(&slots).acquire_owned().await
                }, if exited.is_none() && write_failure.is_none() => {
                    let now = tokio::time::Instant::now();
                    let token = deadlines.iter().find(|(_, at)| **at <= now)
                        .map(|(token, _)| token.clone()).expect("expired deadline");
                    deadlines.remove(&token);
                    let message = ClientMessage::Queued {
                        message: Box::new(ClientMessage::Deadline { token }),
                        _permit: permit.context("plugin deadline admission closed")?,
                        _bytes: Arc::clone(&inbound_bytes).try_acquire_many_owned(0)
                            .context("plugin inbound byte admission closed")?,
                    };
                    events.try_send(Event { plugin, result: Ok(message) })
                        .map_err(|_| anyhow::anyhow!("plugin deadline queue full or closed"))?;
                }
                status = &mut child_exit, if exited.is_none() => {
                    let status = status?;
                    if !drain_after_exit {
                        bail!("plugin exited: {status}");
                    }
                    exited = Some(status);
                    deadline = None;
                    exit_drain.get_or_insert_with(|| {
                        tokio::time::Instant::now() + Duration::from_secs(2)
                    });
                }
                _ = async {
                    match exit_drain {
                        Some(deadline) => tokio::time::sleep_until(deadline).await,
                        None => std::future::pending().await,
                    }
                } => {
                    if let Some(error) = write_failure.take() {
                        return Err(error);
                    }
                    bail!("plugin exited: {}", exited.expect("exit drain has status"));
                }
            }
        }
    }
}

async fn write_message<W: AsyncWrite + Unpin>(
    writer: &mut W,
    bytes: &[u8],
    uncertain: Option<&AtomicBool>,
) -> Result<()> {
    if let Some(uncertain) = uncertain {
        uncertain.store(true, Ordering::Release);
    }
    let result = tokio::time::timeout(Duration::from_secs(2), writer.write_all(bytes)).await;
    if matches!(result, Ok(Ok(())))
        && let Some(uncertain) = uncertain
    {
        uncertain.store(false, Ordering::Release);
    }
    result
        .context("plugin stopped reading")?
        .context("plugin input write failed")
}

fn queued(
    message: ClientMessage,
    size: usize,
    slots: &Arc<tokio::sync::Semaphore>,
    bytes: &Arc<tokio::sync::Semaphore>,
) -> Result<ClientMessage> {
    let permit = slots
        .clone()
        .try_acquire_owned()
        .context("plugin inbound queue full")?;
    let bytes = bytes
        .clone()
        .try_acquire_many_owned(size as u32)
        .context("plugin inbound byte quota exceeded")?;
    Ok(ClientMessage::Queued {
        message: Box::new(message),
        _permit: permit,
        _bytes: bytes,
    })
}

#[derive(Debug, Default)]
struct OutputWake {
    // Encoded size plus one; zero means no host state is waiting for capacity.
    waiting: AtomicUsize,
    queued: AtomicBool,
    changed: Notify,
}
impl OutputWake {
    fn arm(&self, size: usize) {
        self.waiting.store(size + 1, Ordering::Release);
        self.changed.notify_one();
    }

    fn ready(&self, capacity: usize, used: usize, limit: usize) -> bool {
        !self.queued.load(Ordering::Acquire)
            && capacity > RESERVED_MESSAGES
            && self
                .waiting
                .load(Ordering::Acquire)
                .checked_sub(1)
                .is_some_and(|size| {
                    used.checked_add(size)
                        .is_some_and(|total| total <= limit.saturating_sub(RESERVED_BYTES))
                })
    }

    fn notification(self: &Arc<Self>) -> OutputReadyGuard {
        self.queued.store(true, Ordering::Release);
        self.waiting.store(0, Ordering::Release);
        OutputReadyGuard(Arc::clone(self))
    }
}

/// At most one queued readiness notice per worker. A host flush that hits
/// capacity again can rearm while this notice is being processed; dropping the
/// guard closes that race even when the worker has already drained its pipe.
#[derive(Debug)]
pub struct OutputReadyGuard(Arc<OutputWake>);
impl Drop for OutputReadyGuard {
    fn drop(&mut self) {
        self.0.queued.store(false, Ordering::Release);
        if self.0.waiting.load(Ordering::Acquire) != 0 {
            self.0.changed.notify_one();
        }
    }
}

pub async fn receive(events: &mut Option<mpsc::Receiver<Event>>) -> Option<Event> {
    match events {
        Some(events) => events.recv().await,
        None => std::future::pending().await,
    }
}

/// Charges encoded payload before admission, including messages waiting for IO.
#[derive(Clone)]
pub struct Sender {
    sender: mpsc::Sender<HostMessage>,
    bytes: Arc<AtomicUsize>,
    limit: usize,
    output: Arc<OutputWake>,
}
impl Sender {
    #[cfg(test)]
    pub fn new(sender: mpsc::Sender<HostMessage>) -> Self {
        Self::bounded(sender, application::MAX_QUEUE_BYTES)
    }
    fn bounded(sender: mpsc::Sender<HostMessage>, limit: usize) -> Self {
        Self {
            sender,
            bytes: Arc::new(AtomicUsize::new(0)),
            limit,
            output: Arc::new(OutputWake::default()),
        }
    }
    /// Check wire size without allocating or admitting an outbound message.
    pub(crate) fn message_fits(message: &HostMessage) -> Result<bool> {
        Ok(encoded_len(message)? <= MAX_BYTES)
    }

    pub fn try_send(&self, message: HostMessage) -> Result<()> {
        let size = encoded_len(&message)?;
        ensure!(size <= MAX_BYTES, "plugin outbound message exceeds limit");
        self.bytes
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |used| {
                used.checked_add(size).filter(|sum| *sum <= self.limit)
            })
            .map_err(|_| anyhow::anyhow!("plugin outbound byte quota exceeded"))?;
        if self.sender.try_send(message).is_err() {
            self.bytes.fetch_sub(size, Ordering::Relaxed);
            bail!("plugin outbound queue full or closed");
        }
        Ok(())
    }

    /// State pressure is nonfatal and arms one capacity-return notification.
    /// Reliable messages keep eight slots and 512 KiB beyond state admission.
    pub fn try_send_state(&self, message: HostMessage) -> Result<bool> {
        let size = encoded_len(&message)?;
        ensure!(size <= MAX_BYTES, "plugin outbound message exceeds limit");
        let permit = match self.sender.try_reserve() {
            Ok(permit) => permit,
            Err(mpsc::error::TrySendError::Full(_)) => {
                self.output.arm(size);
                return Ok(false);
            }
            Err(mpsc::error::TrySendError::Closed(_)) => bail!("plugin outbound queue closed"),
        };
        if self.sender.capacity() < RESERVED_MESSAGES {
            drop(permit);
            self.output.arm(size);
            return Ok(false);
        }
        if self
            .bytes
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |used| {
                used.checked_add(size)
                    .filter(|sum| *sum <= self.limit.saturating_sub(RESERVED_BYTES))
            })
            .is_err()
        {
            drop(permit);
            self.output.arm(size);
            return Ok(false);
        }
        permit.send(message);
        Ok(true)
    }
}
fn encoded_len(message: &HostMessage) -> Result<usize> {
    if matches!(message, HostMessage::Deadline { .. }) {
        return Ok(0);
    }
    struct Counter(usize);
    impl std::io::Write for Counter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0 += bytes.len();
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut count = Counter(1);
    serde_json::to_writer(&mut count, message)?;
    Ok(count.0)
}

#[cfg(test)]
#[path = "tests/worker.rs"]
mod admission_tests;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn queue_counts_encoded_bytes_and_releases_failed_admission() {
        let (tx, _rx) = mpsc::channel(32);
        let sender = Sender::new(tx);
        let large = || {
            HostMessage::Application(application::HostMessage::Response {
                id: "p:1".into(),
                outcome: application::Response::Failure {
                    error: application::Error::new(
                        application::ErrorCode::Internal,
                        &"x".repeat(900_000),
                    ),
                },
            })
        };
        for _ in 0..4 {
            sender.try_send(large()).unwrap();
        }
        assert!(sender.try_send(large()).is_err());
        assert!(sender.bytes.load(Ordering::Relaxed) <= application::MAX_QUEUE_BYTES);
        let (tx, rx) = mpsc::channel(1);
        let sender = Sender::new(tx);
        drop(rx);
        assert!(sender.try_send(large()).is_err());
        assert_eq!(sender.bytes.load(Ordering::Relaxed), 0);
    }
}
