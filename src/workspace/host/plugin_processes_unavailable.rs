// SPDX-License-Identifier: MPL-2.0

//! Deferred native helpers have no worker, handle or event producer.
use super::WorkspaceHost;
use crate::plugin::{application as api, process};

pub(super) struct Managed {
    pub(super) owner: usize,
    _uninhabited: std::convert::Infallible,
}

pub(super) fn is_process_request(request: &api::Request) -> bool {
    matches!(
        request,
        api::Request::ProcessStart(_)
            | api::Request::ProcessGet { .. }
            | api::Request::ProcessRead { .. }
            | api::Request::ProcessWrite { .. }
            | api::Request::ProcessClose { .. }
    )
}

impl WorkspaceHost {
    pub(super) fn pending_process_requests(&self, _owner: Option<usize>) -> usize {
        0
    }
    pub(super) fn process_info(&self, _owner: usize, _handle: &str) -> Option<&process::Info> {
        None
    }
    pub(super) fn process_observation_info(
        &self,
        _owner: usize,
        _handle: &str,
    ) -> Option<&process::Info> {
        None
    }
    pub(super) fn application_process_request(
        &mut self,
        _owner: usize,
        _request: &str,
        _operation: api::Request,
    ) -> Result<Option<api::ResultValue>, api::Error> {
        Err(api::Error::new(
            api::ErrorCode::Unavailable,
            "Plugin processes are unavailable on this platform",
        ))
    }
    pub(super) fn stop_plugin_processes(&mut self, _owner: usize) {}
    pub(super) fn application_process_event(
        &mut self,
        _owner: usize,
        event: process::runtime::Event,
    ) -> anyhow::Result<()> {
        match event {}
    }
}
