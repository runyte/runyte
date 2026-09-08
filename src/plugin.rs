// SPDX-License-Identifier: MPL-2.0

//! Experimental process extension API. The wire contract is documented in
//! `docs/plugins.md`; these service internals are not a stable Rust API.

use std::{collections::BTreeMap, path::PathBuf, process::Stdio, time::Duration};

use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    sync::mpsc,
};

pub const VERSION: &str = "runyte-experimental-1";
pub const MAX_BYTES: usize = 1_048_576;
pub const MAX_SELECTIONS: usize = 1024;
pub const MAX_PLUGINS: usize = 8;
pub const MAX_COMMANDS: usize = 16;
pub const TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginConfig {
    pub id: String,
    #[serde(default)]
    pub enabled: bool,
    pub executable: PathBuf,
    #[serde(default)]
    pub args: Vec<String>,
    /// Local command name to physical key sequence, Normal and Select only.
    #[serde(default)]
    pub bindings: BTreeMap<String, String>,
}

pub fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 48
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Registration {
    pub name: String,
    pub description: String,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ClientMessage {
    Register {
        version: String,
        commands: Vec<Registration>,
    },
    Replace {
        invocation: String,
        replacements: Vec<String>,
    },
    Fail {
        invocation: String,
        message: String,
    },
    Subscribe {
        request: String,
        buffer: String,
    },
    Unsubscribe {
        request: String,
        buffer: String,
    },
}

#[derive(Clone, Debug, Serialize)]
pub struct Selection {
    pub anchor: usize,
    pub head: usize,
    pub from: usize,
    pub to: usize,
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum HostMessage {
    Hello {
        version: &'static str,
    },
    Registered {
        commands: Vec<String>,
    },
    Invoke {
        invocation: String,
        command: String,
        buffer: String,
        revision: String,
        text: String,
        selections: Vec<Selection>,
        primary: usize,
    },
    Complete {
        invocation: String,
        status: &'static str,
        revision: Option<String>,
        message: String,
    },
    Subscribed {
        request: String,
        buffer: String,
        revision: String,
    },
    Unsubscribed {
        request: String,
        buffer: String,
    },
    Error {
        request: String,
        code: &'static str,
    },
    BufferState {
        sequence: String,
        buffer: String,
        revision: String,
        closed: bool,
    },
}

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
) -> (Worker, mpsc::Sender<HostMessage>) {
    let (sender, receiver) = mpsc::channel(8);
    let worker = tokio::spawn(async move {
        if let Err(error) = run(config, root, plugin, &events, receiver).await {
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
) -> Result<ClientMessage> {
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
    serde_json::from_slice(&bytes).context("invalid plugin message")
}

async fn run(
    config: PluginConfig,
    root: PathBuf,
    plugin: usize,
    events: &mpsc::Sender<Event>,
    mut input: mpsc::Receiver<HostMessage>,
) -> Result<()> {
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
    // Keep the partially read message future across outbound messages: cancelling
    // read_until would lose bytes already consumed from the pipe.
    loop {
        let mut reading = Box::pin(read_message(&mut reader));
        loop {
            tokio::select! {
                result = &mut reading => {
                    let message = result?;
                    events.try_send(Event { plugin, result: Ok(message) })
                        .map_err(|_| anyhow::anyhow!("plugin inbound queue full or closed"))?;
                    break;
                }
                message = input.recv() => {
                    let Some(message) = message else { return Ok(()); };
                    match &message {
                        HostMessage::Registered { .. } | HostMessage::Complete { .. } => deadline = None,
                        HostMessage::Invoke { .. } => deadline = Some(tokio::time::Instant::now() + TIMEOUT),
                        _ => {}
                    }
                    let mut bytes = serde_json::to_vec(&message)?;
                    ensure!(bytes.len() < MAX_BYTES, "plugin outbound message exceeds limit");
                    bytes.push(b'\n');
                    tokio::time::timeout(Duration::from_secs(2), writer.write_all(&bytes)).await
                        .context("plugin stopped reading")??;
                }
                _ = async { match deadline {
                    Some(deadline) => tokio::time::sleep_until(deadline).await,
                    None => std::future::pending().await,
                }} => bail!("plugin registration or invocation timed out"),
                status = child.wait() => bail!("plugin exited: {}", status?),
            }
        }
    }
}

pub async fn receive(events: &mut Option<mpsc::Receiver<Event>>) -> Option<Event> {
    match events {
        Some(events) => events.recv().await,
        None => std::future::pending().await,
    }
}
