// SPDX-License-Identifier: MPL-2.0

//! Explicit foreground handoffs to native owners.
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TerminalOpen {
    pub invocation: String,
    pub label: String,
    pub executable: String,
    #[serde(default, deserialize_with = "super::process::arguments")]
    pub args: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
}
impl TerminalOpen {
    pub fn validate(&self) -> Result<(), super::application::Error> {
        super::process::validate_launch(
            &self.label,
            &self.executable,
            &self.args,
            self.cwd.as_deref(),
        )
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Target {
    Url { url: String },
    File { path: String },
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Info,
    Warning,
    Error,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Notification {
    pub severity: Severity,
    pub title: String,
    pub body: String,
}

pub(crate) struct Lease {
    pub owner: usize,
    pub generation: String,
    pub request: String,
    pub sender: tokio::sync::mpsc::Sender<super::Event>,
    pub permit: Option<tokio::sync::OwnedSemaphorePermit>,
}
impl std::fmt::Debug for Lease {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HandoffLease").finish_non_exhaustive()
    }
}
impl Drop for Lease {
    fn drop(&mut self) {
        let Some(permit) = self.permit.take() else {
            return;
        };
        // The preceding result was consumed before its last lease can drop.
        // The shared local-work permit reserves this one internal queue slot.
        let _ = self.sender.try_send(super::Event {
            plugin: self.owner,
            result: Ok(super::ClientMessage::Handoff(Event {
                generation: self.generation.clone(),
                request: self.request.clone(),
                kind: Kind::Settled { _permit: permit },
            })),
        });
    }
}

#[derive(Debug)]
pub(crate) enum Prepared {
    Terminal(Box<crate::terminal::PendingTerminal>),
    External(crate::external_open::system::Prepared),
    Launched,
}
#[derive(Debug)]
pub(crate) enum Kind {
    Prepared {
        result: Result<Prepared, super::application::Error>,
        lease: std::sync::Arc<Lease>,
    },
    Settled {
        _permit: tokio::sync::OwnedSemaphorePermit,
    },
}
#[derive(Debug)]
pub struct Event {
    pub(crate) generation: String,
    pub(crate) request: String,
    pub(crate) kind: Kind,
}
