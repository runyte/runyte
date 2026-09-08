// SPDX-License-Identifier: MPL-2.0

//! Metadata-only observations at host mutation checkpoints; no polling task.
use super::WorkspaceHost;
use crate::plugin::{application as api, observation as wire};
use api::{Error, ErrorCode as Code};

const OBSERVATION_CHARGE: usize = 8 * 1024 * 1024;

impl WorkspaceHost {
    fn validate_observation_source(
        &self,
        owner: usize,
        source: &wire::Source,
    ) -> Result<(), Error> {
        let state = &self.app.plugins.instances[&owner].application;
        let (capability, owned, closed) = match source {
            wire::Source::Buffer { buffer } => (
                "workspace",
                state.buffers.contains_key(buffer),
                state
                    .buffers
                    .get(buffer)
                    .is_some_and(|&index| self.app.host_buffer_is_closed(index)),
            ),
            wire::Source::Pane { pane } => (
                "workspace",
                state.panes.contains_key(pane),
                state
                    .panes
                    .get(pane)
                    .is_some_and(|index| !self.app.panes.contains_key(index)),
            ),
            wire::Source::View { view } | wire::Source::ViewActions { view } => (
                "views",
                state.views.contains_key(view),
                state
                    .views
                    .get(view)
                    .is_some_and(|view| self.app.host_buffer_is_closed(view.buffer)),
            ),
            wire::Source::Viewport { view, pane } => (
                "views",
                state.views.contains_key(view) && state.panes.contains_key(pane),
                state
                    .views
                    .get(view)
                    .is_some_and(|view| self.app.host_buffer_is_closed(view.buffer))
                    || state
                        .panes
                        .get(pane)
                        .is_some_and(|pane| !self.app.panes.contains_key(pane)),
            ),
            wire::Source::Job { job } => ("jobs", state.jobs.contains_key(job), false),
            wire::Source::Process { process } => (
                "processes",
                state.processes.contains(process),
                self.process_info(owner, process).is_none(),
            ),
            wire::Source::Attachment | wire::Source::Buffers => ("workspace", true, false),
        };
        if !state.capabilities.contains(capability) {
            return Err(Error::new(
                Code::CapabilityDenied,
                "Source capability was not granted",
            ));
        }
        if !owned {
            return Err(Error::new(Code::NotFound, "Unknown observation source"));
        }
        if closed {
            return Err(Error::new(Code::Closed, "Observation source is closed"));
        }
        Ok(())
    }

    fn observation_snapshot(
        &mut self,
        owner: usize,
        source: &wire::Source,
    ) -> Result<wire::Snapshot, Error> {
        let state = &self.app.plugins.instances[&owner].application;
        Ok(match source {
            wire::Source::Buffer { buffer } => {
                let Some(&index) = state.buffers.get(buffer) else {
                    return Ok(wire::Snapshot::Closed {});
                };
                if self.app.host_buffer_is_closed(index) {
                    wire::Snapshot::Closed {}
                } else {
                    let buffer = &self.app.buffers[index];
                    wire::Snapshot::Buffer {
                        revision: format!("r:{}", buffer.revision()),
                        saved_revision: buffer
                            .plugin_saved_revision()
                            .map(|revision| format!("r:{revision}")),
                        name: observation_name(&buffer.display_name()),
                        chars: buffer.len_chars(),
                        read_only: buffer.is_read_only(),
                        dirty: buffer.dirty,
                    }
                }
            }
            wire::Source::Pane { pane } => {
                let Some(&index) = state.panes.get(pane) else {
                    return Ok(wire::Snapshot::Closed {});
                };
                let Some(pane) = self.app.panes.get(&index) else {
                    return Ok(wire::Snapshot::Closed {});
                };
                let buffer = pane.terminal.is_none().then_some(pane.buffer);
                let selection_revision = format!("q:{}", self.app.plugin_selection_revision(index));
                let buffer = buffer
                    .map(|index| {
                        self.app
                            .plugins
                            .instances
                            .get_mut(&owner)
                            .unwrap()
                            .application
                            .buffer_handle(index)
                    })
                    .transpose()?;
                wire::Snapshot::Pane {
                    buffer,
                    selection_revision,
                }
            }
            wire::Source::View { view } => {
                match state
                    .views
                    .get(view)
                    .filter(|view| !self.app.host_buffer_is_closed(view.buffer))
                {
                    Some(view) => wire::Snapshot::View {
                        revision: format!("m:{}", view.revision),
                        query: view
                            .query
                            .as_ref()
                            .map(crate::plugin::view::QueryState::wire),
                    },
                    None => wire::Snapshot::Closed {},
                }
            }
            wire::Source::ViewActions { view } => {
                match state
                    .views
                    .get(view)
                    .filter(|view| !self.app.host_buffer_is_closed(view.buffer))
                {
                    Some(view) => wire::Snapshot::ViewActions {
                        accepted: format!("a:{}", view.accepted_actions),
                    },
                    None => wire::Snapshot::Closed {},
                }
            }
            wire::Source::Viewport { view, pane } => {
                match (state.views.get(view), state.panes.get(pane)) {
                    (Some(live), Some(&pane))
                        if !self.app.host_buffer_is_closed(live.buffer)
                            && self.app.panes.contains_key(&pane) =>
                    {
                        self.app
                            .plugin_viewport(owner, view, pane)
                            .cloned()
                            .unwrap_or(wire::Snapshot::Viewport {
                                model_revision: None,
                                visible: false,
                                top: None,
                                bottom: None,
                            })
                    }
                    _ => wire::Snapshot::Closed {},
                }
            }
            wire::Source::Process { process } => {
                match self.process_observation_info(owner, process) {
                    Some(info) => wire::Snapshot::Process {
                        state: info.state,
                        stdout: info.stdout,
                        stderr: info.stderr,
                        stdin_closed: info.stdin_closed,
                        output_truncated: info.output_truncated,
                        exit_code: info.exit_code,
                        signal: info.signal,
                    },
                    None => wire::Snapshot::Closed {},
                }
            }
            wire::Source::Job { job } => match state.jobs.get(job) {
                Some(job) => wire::Snapshot::Job {
                    state: job.state,
                    progress: job.progress,
                },
                None => wire::Snapshot::Closed {},
            },
            wire::Source::Buffers => {
                return Err(Error::new(
                    Code::InvalidArgument,
                    "Discovery filters must be expanded",
                ));
            }
            wire::Source::Attachment => wire::Snapshot::Attachment {
                attached: self.app.plugins.frontend_attached,
                generation: self.app.plugins.attachment_generation.to_string(),
            },
        })
    }
}

impl WorkspaceHost {
    pub(super) fn application_observation_request(
        &mut self,
        owner: usize,
        request_id: String,
        request: api::Request,
    ) -> anyhow::Result<()> {
        if let api::Request::EventUnsubscribe { subscription } = request {
            let state = &mut self
                .app
                .plugins
                .instances
                .get_mut(&owner)
                .unwrap()
                .application;
            let was_empty = state.observations.is_empty();
            let deliveries = state.observations.unsubscribe(&subscription);
            if !was_empty && state.observations.is_empty() {
                state.retained_payload -= OBSERVATION_CHARGE;
            }
            // Anything already admitted stays before the response in the same
            // FIFO. Retained final observations are flushed before this ack.
            for delivery in deliveries {
                self.send_observation(owner, delivery, true)?;
            }
            self.refresh_plugin_viewport_watches(None);
            return self.application_local_reply(
                owner,
                request_id,
                Ok(api::ResultValue::Empty(api::Empty {})),
            );
        }
        if matches!(request, api::Request::EventResync { .. }) {
            self.collect_observations(owner)
                .map_err(|error| anyhow::anyhow!(error.message))?;
            self.flush_observations(owner)?;
        }
        let result = self.observation_baseline(owner, request);
        self.application_local_reply(owner, request_id, result)
    }

    fn observation_baseline(
        &mut self,
        owner: usize,
        request: api::Request,
    ) -> Result<api::ResultValue, Error> {
        let (filters, previous) = match request {
            api::Request::EventSubscribe { sources } => {
                if sources.is_empty() || sources.len() > 256 {
                    return Err(Error::new(
                        Code::InvalidArgument,
                        "Subscription needs 1–256 source filters",
                    ));
                }
                for source in &sources {
                    self.validate_observation_source(owner, source)?;
                }
                (sources, None)
            }
            api::Request::EventResync { subscription } => {
                let filters = self.app.plugins.instances[&owner]
                    .application
                    .observations
                    .filters(&subscription)
                    .ok_or_else(|| Error::new(Code::NotFound, "Unknown subscription"))?;
                (filters, Some(subscription))
            }
            _ => unreachable!(),
        };
        let first = self.app.plugins.instances[&owner]
            .application
            .observations
            .is_empty();
        if first {
            self.reserve_application_payload(owner, OBSERVATION_CHARGE)?;
        }
        // Failed admission cannot exhaust editor handles through speculative
        // wildcard expansion or a newly observed pane target.
        let old_buffers = self.app.plugins.instances[&owner]
            .application
            .buffers
            .clone();
        self.refresh_plugin_viewport_watches(Some((owner, &filters)));
        if let Some(prepared) = &self.prepared {
            self.app.capture_plugin_viewports(&prepared.view);
        }
        let baseline = self.capture_observation_baseline(owner, &filters);
        let result = baseline.and_then(|baseline| {
            let encoded = serde_json::to_vec(&baseline)
                .map_err(|_| Error::new(Code::Internal, "Observation baseline encoding failed"))?;
            if encoded.len() + baseline.len() * 128 > crate::plugin::MAX_BYTES - 4096 {
                return Err(Error::new(
                    Code::LimitExceeded,
                    "Observation baseline exceeds the wire limit",
                ));
            }

            let state = &mut self
                .app
                .plugins
                .instances
                .get_mut(&owner)
                .unwrap()
                .application;
            let (subscription, sources) = if let Some(subscription) = previous {
                let sources = state.observations.resync(&subscription, baseline)?;
                (subscription, sources)
            } else {
                state.next_handle += 1;
                let subscription = format!("o:{}:{}", state.generation, state.next_handle);
                let sources =
                    state
                        .observations
                        .subscribe(subscription.clone(), filters, baseline)?;
                (subscription, sources)
            };
            if first {
                state.retained_payload += OBSERVATION_CHARGE;
            }
            state.sequence += 1;
            Ok(api::ResultValue::Observation(wire::Baseline {
                subscription,
                sequence: format!("e:{}", state.sequence),
                sources,
            }))
        });
        if result.is_err() {
            self.app
                .plugins
                .instances
                .get_mut(&owner)
                .unwrap()
                .application
                .buffers = old_buffers;
        }
        self.refresh_plugin_viewport_watches(None);
        result
    }

    /// Watch membership changes only at subscription and owner lifecycle boundaries.
    pub(super) fn refresh_plugin_viewport_watches(
        &mut self,
        extra: Option<(usize, &[wire::Source])>,
    ) {
        let mut watches = std::collections::BTreeSet::new();
        for (&owner, instance) in &self.app.plugins.instances {
            let sources = instance.application.observations.watched_sources();
            let proposed = extra
                .filter(|(id, _)| *id == owner)
                .map_or(&[][..], |(_, sources)| sources);
            for source in sources.iter().chain(proposed) {
                if let wire::Source::Viewport { view, pane } = source
                    && let Some(&pane) = instance.application.panes.get(pane)
                    && self.app.panes.contains_key(&pane)
                    && instance
                        .application
                        .views
                        .get(view)
                        .is_some_and(|live| !self.app.host_buffer_is_closed(live.buffer))
                {
                    watches.insert((owner, view.clone(), pane));
                }
            }
        }
        self.app.set_plugin_viewport_watches(watches);
    }

    fn observation_buffer_indices(&mut self) -> Vec<usize> {
        let revision = self.app.plugin_buffer_membership_revision();
        if self
            .observation_buffers
            .as_ref()
            .is_none_or(|(cached, _)| *cached != revision)
        {
            let indices = if let Some((previous, indices)) = &self.observation_buffers
                && indices.len() <= 256
                && previous.0 <= revision.0
                && previous.1 <= revision.1
            {
                // The previous bounded list contained every live slot. Since
                // IDs never reopen, only retirement and appended slots matter.
                indices
                    .iter()
                    .copied()
                    .filter(|&index| !self.app.host_buffer_is_closed(index))
                    .chain(
                        (previous.0..revision.0)
                            .filter(|&index| !self.app.host_buffer_is_closed(index)),
                    )
                    .take(257)
                    .collect()
            } else {
                // Initial discovery, or an explicit baseline retry after an
                // oversized discovery. Ordinary watched edits never scan here.
                (0..self.app.buffers.len())
                    .filter(|&index| !self.app.host_buffer_is_closed(index))
                    .take(257)
                    .collect()
            };
            self.observation_buffers = Some((revision, indices));
        }
        self.observation_buffers.as_ref().unwrap().1.clone()
    }

    fn capture_observation_baseline(
        &mut self,
        owner: usize,
        filters: &[wire::Source],
    ) -> Result<Vec<(wire::Source, wire::Snapshot)>, Error> {
        let mut sources = std::collections::BTreeSet::new();
        for source in filters {
            if matches!(source, wire::Source::Buffers) {
                let indices = self.observation_buffer_indices();
                if indices.len() > 256 {
                    return Err(Error::new(
                        Code::LimitExceeded,
                        "Buffer discovery exceeds the source limit",
                    ));
                }
                for index in indices {
                    let buffer = self
                        .app
                        .plugins
                        .instances
                        .get_mut(&owner)
                        .unwrap()
                        .application
                        .buffer_handle(index)?;
                    sources.insert(wire::Source::Buffer { buffer });
                }
            } else {
                sources.insert(source.clone());
            }
            if sources.len() > 256 {
                return Err(Error::new(
                    Code::LimitExceeded,
                    "Observation source limit exceeded",
                ));
            }
        }
        sources
            .into_iter()
            .map(|source| {
                self.observation_snapshot(owner, &source)
                    .map(|snapshot| (source, snapshot))
            })
            .collect()
    }

    fn send_observation(
        &mut self,
        owner: usize,
        delivery: wire::Delivery,
        force_reliable: bool,
    ) -> anyhow::Result<bool> {
        let (event, data, reliable) = match delivery {
            wire::Delivery::Changed(change) => {
                ("event.changed", api::EventData::Observation(change), false)
            }
            wire::Delivery::ReliableChanged(change) => {
                ("event.changed", api::EventData::Observation(change), true)
            }
            wire::Delivery::Closed(change) => {
                ("event.closed", api::EventData::Observation(change), true)
            }
            wire::Delivery::ResyncRequired(marker) => (
                "event.resync_required",
                api::EventData::ObservationResync(marker),
                true,
            ),
            wire::Delivery::Action(action) => (
                "event.action",
                api::EventData::ObservationAction(*action),
                true,
            ),
        };
        let state = &self.app.plugins.instances[&owner].application;
        let sequence = state.sequence + 1;
        let message = api::HostMessage::Event {
            sequence: format!("e:{sequence}"),
            event,
            data,
        };
        if reliable || force_reliable {
            self.application_send(owner, message)?;
        } else if !self.app.plugins.instances[&owner]
            .sender
            .try_send_state(crate::plugin::HostMessage::Application(message))?
        {
            return Ok(false);
        }
        self.app
            .plugins
            .instances
            .get_mut(&owner)
            .unwrap()
            .application
            .sequence = sequence;
        Ok(true)
    }

    fn flush_observations(&mut self, owner: usize) -> anyhow::Result<()> {
        while let Some(delivery) = self.app.plugins.instances[&owner]
            .application
            .observations
            .peek_ready()
        {
            if !self.send_observation(owner, delivery, false)? {
                break;
            }
            self.app
                .plugins
                .instances
                .get_mut(&owner)
                .unwrap()
                .application
                .observations
                .ack_ready();
        }
        Ok(())
    }

    pub(super) fn sync_application_observers(&mut self) {
        let owners = self
            .app
            .plugins
            .instances
            .iter()
            .filter_map(|(&owner, instance)| {
                (!instance.application.observations.is_empty()).then_some(owner)
            })
            .collect::<Vec<_>>();
        for owner in owners {
            if self.collect_observations(owner).is_err() || self.flush_observations(owner).is_err()
            {
                self.stop_plugin(owner, "observation consumer is too slow");
            }
        }
    }

    fn collect_observations(&mut self, owner: usize) -> Result<(), Error> {
        let subscriptions = self.app.plugins.instances[&owner]
            .application
            .observations
            .subscriptions();
        let sources = self.app.plugins.instances[&owner]
            .application
            .observations
            .watched_sources();
        let mut viewport_closed = false;
        for source in sources {
            match self.observation_snapshot(owner, &source) {
                Ok(snapshot) => {
                    viewport_closed |= matches!(source, wire::Source::Viewport { .. })
                        && matches!(snapshot, wire::Snapshot::Closed {});
                    self.app
                        .plugins
                        .instances
                        .get_mut(&owner)
                        .unwrap()
                        .application
                        .observations
                        .observe(&source, snapshot)?;
                }
                Err(_) => {
                    for subscription in &subscriptions {
                        if self.app.plugins.instances[&owner]
                            .application
                            .observations
                            .sources(subscription)
                            .is_some_and(|sources| sources.contains(&source))
                        {
                            self.app
                                .plugins
                                .instances
                                .get_mut(&owner)
                                .unwrap()
                                .application
                                .observations
                                .invalidate(subscription)?;
                        }
                    }
                }
            }
        }
        if viewport_closed {
            self.refresh_plugin_viewport_watches(None);
        }
        let discovery = subscriptions
            .into_iter()
            .filter(|subscription| {
                let registry = &self.app.plugins.instances[&owner].application.observations;
                !registry.is_suspended(subscription)
                    && registry
                        .filters(subscription)
                        .is_some_and(|filters| filters.contains(&wire::Source::Buffers))
            })
            .collect::<Vec<_>>();
        if discovery.is_empty() {
            return Ok(());
        }
        // Only wildcard subscriptions inspect live buffer slots. Stop before
        // materializing more than one bounded overflow witness.
        let indices = self.observation_buffer_indices();
        if indices.len() > 256 {
            for subscription in discovery {
                self.app
                    .plugins
                    .instances
                    .get_mut(&owner)
                    .unwrap()
                    .application
                    .observations
                    .invalidate(&subscription)?;
            }
            return Ok(());
        }
        for subscription in discovery {
            for &index in &indices {
                if self.app.plugins.instances[&owner]
                    .application
                    .observations
                    .is_suspended(&subscription)
                {
                    break;
                }
                let source = self
                    .app
                    .plugins
                    .instances
                    .get_mut(&owner)
                    .unwrap()
                    .application
                    .buffer_handle(index)
                    .map(|buffer| wire::Source::Buffer { buffer });
                let snapshot = source.and_then(|source| {
                    self.observation_snapshot(owner, &source)
                        .map(|snapshot| (source, snapshot))
                });
                match snapshot {
                    Ok((source, snapshot)) => self
                        .app
                        .plugins
                        .instances
                        .get_mut(&owner)
                        .unwrap()
                        .application
                        .observations
                        .discover(&subscription, source, snapshot)?,
                    Err(_) => self
                        .app
                        .plugins
                        .instances
                        .get_mut(&owner)
                        .unwrap()
                        .application
                        .observations
                        .invalidate(&subscription)?,
                }
            }
        }
        Ok(())
    }
}

/// Presentation labels are not source identities. Keep them bounded without
/// letting control characters create a second visual record in consumers.
fn observation_name(value: &str) -> String {
    let mut name = String::new();
    for character in value.chars() {
        let text = if character.is_control() {
            character.escape_default().to_string()
        } else {
            character.to_string()
        };
        if name.len() + text.len() > 512 {
            break;
        }
        name.push_str(&text);
    }
    name
}
