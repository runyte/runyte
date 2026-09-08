// SPDX-License-Identifier: MPL-2.0

use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::PathBuf, time::Duration};

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
    pub api: super::application::Api,
    #[serde(default)]
    pub capabilities: Vec<String>,
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
    #[serde(skip)]
    Queued {
        message: Box<ClientMessage>,
        _permit: tokio::sync::OwnedSemaphorePermit,
        _bytes: tokio::sync::OwnedSemaphorePermit,
    },
    #[serde(skip)]
    Application(super::application::ClientMessage),
    #[serde(skip)]
    Unsupported {
        id: String,
    },
    #[serde(skip)]
    Deadline {
        token: String,
    },
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
    #[serde(skip)]
    Deadline {
        token: String,
        after_ms: Option<u64>,
    },
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
    #[serde(untagged)]
    Application(super::application::HostMessage),
}
