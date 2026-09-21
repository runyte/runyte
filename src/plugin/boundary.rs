// SPDX-License-Identifier: MPL-2.0

use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::PathBuf, time::Duration};

pub const VERSION: &str = super::application::VERSION;
pub const MAX_BYTES: usize = 1_048_576;
pub const MAX_PLUGINS: usize = 8;
pub const MAX_COMMANDS: usize = super::application::MAX_COMMANDS;
pub const TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PluginConfig {
    pub id: String,
    #[serde(default)]
    pub settings: super::settings::Settings,
    #[serde(default)]
    pub api: String,
    #[serde(default)]
    pub runyte: String,
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

#[derive(Debug)]
pub enum ClientMessage {
    ProviderReload(super::provider::ReloadEvent),
    WorkerStopped {
        failure: Option<String>,
        reaped: bool,
    },
    Process(super::process::runtime::Event),
    State(super::state::Event),
    Handoff(super::handoff::Event),
    ModelPrepared {
        generation: String,
        request: String,
        result: Result<super::view::Prepared, super::application::Error>,
        _permit: tokio::sync::OwnedSemaphorePermit,
    },
    OutputReady {
        _notification: super::OutputReadyGuard,
    },
    FilesystemApplied {
        job: String,
        result: Result<super::filesystem::Applied, String>,
        _permit: tokio::sync::OwnedSemaphorePermit,
    },
    DocumentSaved {
        job: String,
        result: Result<Option<crate::buffer::SavedDocument>, String>,
        _permit: tokio::sync::OwnedSemaphorePermit,
    },
    Local {
        generation: String,
        request: String,
        result: Result<super::filesystem::Prepared, super::application::Error>,
        _permit: tokio::sync::OwnedSemaphorePermit,
    },
    Queued {
        message: Box<ClientMessage>,
        _permit: tokio::sync::OwnedSemaphorePermit,
        _bytes: tokio::sync::OwnedSemaphorePermit,
    },
    Application(super::application::ClientMessage),
    Unsupported {
        id: String,
    },
    Deadline {
        token: String,
    },
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
// The bounded outbound queue predominantly carries application frames; avoid an
// extra allocation for every ordinary frame just to shrink the timer variant.
#[allow(clippy::large_enum_variant)]
pub enum HostMessage {
    #[serde(skip)]
    Deadline {
        token: String,
        after_ms: Option<u64>,
    },
    #[serde(untagged)]
    Application(super::application::HostMessage),
}
