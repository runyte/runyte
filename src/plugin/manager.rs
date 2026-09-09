// SPDX-License-Identifier: MPL-2.0

//! Native manager values. These are editor state, not a plugin wire API.

pub(crate) const MAX_CONFIGS: usize = 128;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Phase {
    Disabled,
    Stopped,
    Starting,
    Running,
    Stopping,
    RestartPending,
    Failed,
}

impl Phase {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Disabled => "Disabled",
            Self::Stopped => "Stopped",
            Self::Starting => "Starting",
            Self::Running => "Running",
            Self::Stopping => "Stopping",
            Self::RestartPending => "Restart pending",
            Self::Failed => "Failed",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Entry {
    pub config_index: usize,
    pub configured_id: String,
    pub enabled: bool,
    pub valid: bool,
    pub phase: Phase,
    pub owner: Option<usize>,
    pub granted: Vec<String>,
    pub jobs: usize,
    pub activities: usize,
    pub helpers: usize,
    pub cleanup: usize,
    pub diagnostic: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Action {
    Stop,
    Restart,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Intent {
    pub config_index: usize,
    pub expected_owner: Option<usize>,
    pub action: Action,
}
