// SPDX-License-Identifier: MPL-2.0

use super::*;
use anyhow::{Context, Result, bail, ensure};
use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};
use std::{path::PathBuf, process::Stdio, time::Duration};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    sync::{Notify, mpsc},
};

const PRODUCER_EVENTS: usize = 16;
const RESERVED_MESSAGES: usize = 8;
const RESERVED_BYTES: usize = 512 * 1024;
/// Each worker owns 16 input/deadline permits, one state-ready notice and one
/// failure. Local IO retains its separate 16 permits until its result is read.
/// Each of 32 managed helpers reserves three ordinary events and one final reap.
pub const EVENT_CAPACITY: usize = MAX_PLUGINS * (PRODUCER_EVENTS + 2) + 16 + 32 * 4;

#[derive(Debug)]
pub struct Event {
    pub plugin: usize,
    pub result: Result<ClientMessage, String>,
}

/// A worker owns the process and all its pipes. Dropping the host aborts it;
/// Tokio's kill-on-drop child handles termination/reaping without blocking UI.
pub struct Worker(tokio::task::JoinHandle<()>);
impl Drop for Worker {
    fn drop(&mut self) {
        self.0.abort();
    }
}

pub fn spawn(
    config: PluginConfig,
    root: PathBuf,
    plugin: usize,
    events: mpsc::Sender<Event>,
) -> (Worker, Sender) {
    let (sender, receiver) = mpsc::channel(if config.api == application::Api::Epoch2 {
        32
    } else {
        8
    });
    let sender = Sender::bounded(
        sender,
        if config.api == application::Api::Epoch2 {
            application::MAX_QUEUE_BYTES
        } else {
            MAX_BYTES * 8
        },
    );
    let admission = OutputAdmission {
        charge: Arc::clone(&sender.bytes),
        output: Arc::clone(&sender.output),
        limit: sender.limit,
    };
    let worker = tokio::spawn(async move {
        if let Err(error) = run(config, root, plugin, &events, receiver, admission).await {
            let _ = events
                .send(Event {
                    plugin,
                    result: Err(error.to_string()),
                })
                .await;
        }
    });
    (Worker(worker), sender)
}

async fn read_message(
    reader: &mut BufReader<tokio::process::ChildStdout>,
    api: application::Api,
) -> Result<(ClientMessage, usize)> {
    let mut bytes = Vec::new();
    let n = reader
        .take((MAX_BYTES + 1) as u64)
        .read_until(b'\n', &mut bytes)
        .await?;
    ensure!(n > 0, "plugin closed stdout");
    ensure!(
        n <= MAX_BYTES && bytes.last() == Some(&b'\n'),
        "plugin message exceeds limit or lacks newline"
    );
    match api {
        application::Api::Epoch1 => serde_json::from_slice(&bytes)
            .map(|message| (message, n))
            .context("invalid plugin message"),
        application::Api::Epoch2 => application::decode(&bytes)
            .map(|message| (message, n))
            .context("invalid application message"),
    }
}

struct OutputAdmission {
    charge: Arc<AtomicUsize>,
    output: Arc<OutputWake>,
    limit: usize,
}

async fn run(
    config: PluginConfig,
    root: PathBuf,
    plugin: usize,
    events: &mpsc::Sender<Event>,
    mut input: mpsc::Receiver<HostMessage>,
    admission: OutputAdmission,
) -> Result<()> {
    let OutputAdmission {
        charge,
        output,
        limit,
    } = admission;
    let mut child = tokio::process::Command::new(&config.executable)
        .args(&config.args)
        .current_dir(root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .context("cannot start plugin")?;
    let mut writer = child.stdin.take().expect("piped stdin");
    let mut reader = BufReader::new(child.stdout.take().expect("piped stdout"));
    let mut deadline = Some(tokio::time::Instant::now() + TIMEOUT);
    let slots = Arc::new(tokio::sync::Semaphore::new(PRODUCER_EVENTS));
    let inbound_bytes = Arc::new(tokio::sync::Semaphore::new(application::MAX_QUEUE_BYTES));
    let mut deadlines = BTreeMap::<String, tokio::time::Instant>::new();
    // Keep the partially read message future across outbound messages: cancelling
    // read_until would lose bytes already consumed from the pipe.
    loop {
        let mut reading = Box::pin(read_message(&mut reader, config.api));
        loop {
            tokio::select! {
                result = &mut reading => {
                    let (message, size) = result?;
                    // Both epochs share producer admission. Internal deadlines
                    // consume these same permits rather than another owner's
                    // reserved space in the host channel.
                    let message = queued(message, size, &slots, &inbound_bytes)?;
                    events.try_send(Event { plugin, result: Ok(message) })
                        .map_err(|_| anyhow::anyhow!("plugin inbound queue full or closed"))?;
                    break;
                }
                message = input.recv() => {
                    let Some(message) = message else { return Ok(()); };
                    charge.fetch_sub(encoded_len(&message)?, Ordering::Relaxed);
                    if let HostMessage::Deadline { token, after_ms } = message {
                        if let Some(ms) = after_ms { deadlines.insert(token, tokio::time::Instant::now() + Duration::from_millis(ms)); }
                        else { deadlines.remove(&token); }
                        continue;
                    }
                    match &message {
                        HostMessage::Application(application::HostMessage::Registered { .. }) | HostMessage::Registered { .. } | HostMessage::Complete { .. } => deadline = None,
                        HostMessage::Invoke { .. } => deadline = Some(tokio::time::Instant::now() + TIMEOUT),
                        _ => {}
                    }
                    let mut bytes = serde_json::to_vec(&message)?;
                    ensure!(bytes.len() < MAX_BYTES, "plugin outbound message exceeds limit");
                    bytes.push(b'\n');
                    tokio::time::timeout(Duration::from_secs(2), writer.write_all(&bytes)).await
                        .context("plugin stopped reading")?.context("plugin input write failed")?;
                }
                _ = output.changed.notified() => {}
                permit = events.reserve(), if output.ready(input.capacity(), charge.load(Ordering::Acquire), limit) => {
                    let permit = permit.context("plugin event channel closed")?;
                    let notification = output.notification();
                    permit.send(Event { plugin, result: Ok(ClientMessage::OutputReady { _notification: notification }) });
                }
                _ = async { match deadline {
                    Some(deadline) => tokio::time::sleep_until(deadline).await,
                    None => std::future::pending().await,
                }} => bail!("plugin registration or invocation timed out"),
                permit = async {
                    match deadlines.values().min().copied() {
                        Some(at) => tokio::time::sleep_until(at).await,
                        None => std::future::pending().await,
                    }
                    // Host timers can expire together without an overloaded
                    // plugin. Wait for this owner's admission inside select so
                    // pipe IO and other owners remain responsive meanwhile.
                    Arc::clone(&slots).acquire_owned().await
                } => {
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
                status = child.wait() => bail!("plugin exited: {}", status?),
            }
        }
    }
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
        let large = || HostMessage::Complete {
            invocation: "1".into(),
            status: "failed",
            revision: None,
            message: "x".repeat(900_000),
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
