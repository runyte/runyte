// SPDX-License-Identifier: MPL-2.0

//! Deferred plugin handoffs cannot reserve native resources.
use super::WorkspaceHost;
use crate::plugin::{application as api, handoff};

pub(super) enum Pending {}

pub(super) fn is_handoff_request(request: &api::Request) -> bool {
    matches!(
        request,
        api::Request::TerminalOpen(_) | api::Request::ExternalOpen { .. }
    )
}

impl WorkspaceHost {
    pub(super) fn application_handoff_request(
        &mut self,
        _owner: usize,
        _request: &str,
        _operation: api::Request,
    ) -> Result<(), api::Error> {
        Err(api::Error::new(
            api::ErrorCode::Unavailable,
            "Plugin handoffs are unavailable on this platform",
        ))
    }
    pub(super) fn application_handoff_event(
        &mut self,
        _owner: usize,
        event: handoff::Event,
    ) -> anyhow::Result<()> {
        match event {}
    }
    pub(super) fn stop_plugin_handoffs(&mut self, _owner: usize) {}
    pub(super) fn sync_plugin_handoffs(&self) {}
}
