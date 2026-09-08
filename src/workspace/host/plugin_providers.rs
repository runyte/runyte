// SPDX-License-Identifier: MPL-2.0

use super::WorkspaceHost;
use crate::app::plugin_providers::ProviderInspectIntent;
use crate::{
    buffer::{Buffer, ProviderDocument, ProviderIdentity},
    plugin::{self, application as api, provider as wire},
};
use api::{Error, ErrorCode as Code};

pub(super) struct PendingRead {
    pub charge: usize,
    pub rebind: Option<usize>,
    pub inspect: Option<ProviderInspectIntent>,
    rebind_epoch: Option<u64>,
    reconcile_write: Option<String>,
    pub requester: usize,
    pub requester_generation: String,
    pub owner: usize,
    pub generation: String,
    pub identity: ProviderIdentity,
    pub request: String,
    pub metadata: Option<wire::Metadata>,
    pub text: String,
    pub context: Option<api::CapturedContext>,
}

impl WorkspaceHost {
    pub(super) fn application_provider_request(
        &mut self,
        owner: usize,
        request: api::Request,
    ) -> Result<(api::ResultValue, Option<&'static str>), Error> {
        self.application_provider_read(owner, request, None, None)
    }

    fn application_provider_read(
        &mut self,
        owner: usize,
        request: api::Request,
        rebind: Option<usize>,
        inspect: Option<ProviderInspectIntent>,
    ) -> Result<(api::ResultValue, Option<&'static str>), Error> {
        let state = &self.app.plugins.instances[&owner].application;
        match request {
            api::Request::ResourceInspect {
                buffer,
                expected_revision,
                invocation,
            } => {
                if !state.capabilities.contains("documents") || !state.capabilities.contains("jobs")
                {
                    return Err(Error::new(
                        Code::CapabilityDenied,
                        "Resource inspection requires documents and jobs capabilities",
                    ));
                }
                let index = *state
                    .buffers
                    .get(&buffer)
                    .ok_or_else(|| Error::new(Code::NotFound, "Unknown document"))?;
                let context = state
                    .requests
                    .get(&invocation)
                    .ok_or_else(|| Error::new(Code::ContextChanged, "Invoking command finished"))?
                    .clone();
                self.app.plugin_foreground(&context)?;
                if context.buffer != index {
                    return Err(Error::new(
                        Code::ContextChanged,
                        "Inspection must target the invoking document",
                    ));
                }
                if self.app.host_buffer_is_closed(index) {
                    return Err(Error::new(Code::Closed, "Document closed"));
                }
                let revision = self.app.buffers[index].revision();
                if format!("r:{revision}") != expected_revision {
                    return Err(Error::new(Code::Stale, "Document changed"));
                }
                self.start_provider_inspection(
                    owner,
                    ProviderInspectIntent {
                        buffer: index,
                        expected_revision: revision,
                        context,
                    },
                )
            }
            api::Request::ResourceRebind {
                buffer,
                expected_revision,
            } => {
                if !state.capabilities.contains("documents") || !state.capabilities.contains("jobs")
                {
                    return Err(Error::new(
                        Code::CapabilityDenied,
                        "Resource rebind requires documents and jobs capabilities",
                    ));
                }
                let index = *state
                    .buffers
                    .get(&buffer)
                    .ok_or_else(|| Error::new(Code::NotFound, "Unknown document"))?;
                if self.app.host_buffer_is_closed(index) {
                    return Err(Error::new(Code::Closed, "Document closed"));
                }
                if format!("r:{}", self.app.buffers[index].revision()) != expected_revision {
                    return Err(Error::new(Code::Stale, "Document changed"));
                }
                if self.app.document_mutation_pending(index) {
                    return Err(Error::new(Code::Busy, "Document operation is pending"));
                }
                let document = self.app.buffers[index].provider().ok_or_else(|| {
                    Error::new(Code::Unsupported, "Document has no resource provider")
                })?;
                if document.uncertain.is_some() && !self.provider_uncertain.contains_key(&index) {
                    return Err(Error::new(
                        Code::OutcomeUnknown,
                        "Previous write settlement reference is unavailable",
                    ));
                }
                let identity = document.identity.clone();
                let result = self.application_provider_read(
                    owner,
                    api::Request::ResourceOpen {
                        plugin: identity.configured_plugin,
                        provider: identity.provider,
                        key: identity.key,
                        invocation: None,
                    },
                    Some(index),
                    None,
                )?;
                self.app.plugins.document_saves.insert(index);
                Ok(result)
            }
            api::Request::ProviderRegister(registration) => {
                if !state.capabilities.contains("providers") {
                    return Err(Error::new(
                        Code::CapabilityDenied,
                        "Providers capability was not granted",
                    ));
                }
                if !wire::valid_name(&registration.name) {
                    return Err(Error::new(Code::InvalidArgument, "Invalid provider name"));
                }
                if state.providers.contains_key(&registration.name) {
                    return Err(Error::new(Code::Conflict, "Provider is already registered"));
                }
                if state.providers.len() >= 8 {
                    return Err(Error::new(
                        Code::LimitExceeded,
                        "Provider registration limit reached",
                    ));
                }
                self.app
                    .plugins
                    .instances
                    .get_mut(&owner)
                    .unwrap()
                    .application
                    .providers
                    .insert(registration.name.clone(), registration);
                Ok((api::ResultValue::Empty(api::Empty {}), None))
            }
            api::Request::ResourceOpen {
                plugin,
                provider,
                key,
                invocation,
            } => {
                if inspect.is_none()
                    && (!state.capabilities.contains("documents")
                        || !state.capabilities.contains("jobs"))
                {
                    return Err(Error::new(
                        Code::CapabilityDenied,
                        "Resource open requires documents and jobs capabilities",
                    ));
                }
                if !wire::valid_name(&provider)
                    || !wire::safe_text(&key, 4096)
                    || !wire::valid_name(&plugin)
                {
                    return Err(Error::new(
                        Code::InvalidArgument,
                        "Invalid resource identity",
                    ));
                }
                let context = invocation
                    .map(|id| {
                        let context = state.requests.get(&id).ok_or_else(|| {
                            Error::new(Code::ContextChanged, "Invoking command finished")
                        })?;
                        self.app.plugin_foreground(context)?;
                        Ok(context.clone())
                    })
                    .transpose()?;
                let (provider_owner, instance) = self
                    .app
                    .plugins
                    .instances
                    .iter()
                    .find(|(_, instance)| {
                        instance.registered
                            && instance.config.id == plugin
                            && instance.application.providers.contains_key(&provider)
                    })
                    .ok_or_else(|| {
                        Error::new(Code::Unavailable, "Resource provider is unavailable")
                    })?;
                let provider_owner = *provider_owner;
                if instance.application.requests.len() + instance.application.provider_requests
                    >= api::MAX_REQUESTS
                {
                    return Err(Error::new(
                        Code::Busy,
                        "Provider control request limit reached",
                    ));
                }
                let generation = instance.application.generation.clone();
                let requester_generation = state.generation.clone();
                if self
                    .provider_reads
                    .values()
                    .filter(|p| p.requester == owner)
                    .count()
                    >= 2
                    || self
                        .provider_reads
                        .values()
                        .filter(|p| p.owner == provider_owner)
                        .count()
                        >= 2
                {
                    return Err(Error::new(
                        Code::Busy,
                        "Two resource opens are already pending",
                    ));
                }
                // An uncertain write reserves both its captured text and the
                // bounded recovery read, so quota exhaustion cannot trap recovery.
                let charge =
                    if rebind.is_some_and(|index| self.provider_uncertain.contains_key(&index)) {
                        0
                    } else {
                        wire::READ_CHARGE
                    };
                self.reserve_application_payload(owner, charge)?;
                let identity = ProviderIdentity {
                    configured_plugin: plugin,
                    provider,
                    key,
                };
                if self.provider_reads.values().any(|p| p.identity == identity) {
                    return Err(Error::new(Code::Busy, "Resource open is already pending"));
                }
                let job = self.create_provider_job(
                    owner,
                    if inspect.is_some() {
                        "Inspect resource"
                    } else {
                        "Open resource"
                    },
                )?;
                self.app
                    .plugins
                    .instances
                    .get_mut(&owner)
                    .unwrap()
                    .application
                    .retained_payload += charge;
                self.provider_reads.insert(
                    job.job.clone(),
                    PendingRead {
                        charge,
                        rebind,
                        rebind_epoch: rebind
                            .or_else(|| inspect.as_ref().map(|i| i.buffer))
                            .and_then(|index| {
                                self.app.buffers[index].provider().map(|d| d.baseline_epoch)
                            }),
                        reconcile_write: rebind.and_then(|index| {
                            self.provider_uncertain
                                .get(&index)
                                .map(|(_, _, job)| job.clone())
                        }),
                        inspect,
                        requester: owner,
                        requester_generation,
                        owner: provider_owner,
                        generation,
                        identity,
                        request: String::new(),
                        metadata: None,
                        text: String::new(),
                        context,
                    },
                );
                if self.send_provider_read(&job.job).is_err() {
                    let pending = self.provider_reads.remove(&job.job).unwrap();
                    if let Some(index) = pending.rebind {
                        self.app.plugins.document_saves.remove(&index);
                    }
                    if !pending.request.is_empty() {
                        self.app
                            .plugins
                            .instances
                            .get_mut(&provider_owner)
                            .unwrap()
                            .application
                            .provider_requests -= 1;
                    }
                    let state = &mut self
                        .app
                        .plugins
                        .instances
                        .get_mut(&owner)
                        .unwrap()
                        .application;
                    state.retained_payload -= pending.charge;
                    state.jobs.remove(&job.job);
                    let _ = self.plugin_send(
                        owner,
                        plugin::HostMessage::Deadline {
                            token: job.job.clone(),
                            after_ms: None,
                        },
                    );
                    let _ = self.plugin_send(
                        provider_owner,
                        plugin::HostMessage::Deadline {
                            token: pending.request,
                            after_ms: None,
                        },
                    );
                    self.stop_plugin(provider_owner, "resource request consumer is too slow");
                    return Err(Error::new(
                        Code::Unavailable,
                        "Provider queue is unavailable",
                    ));
                }
                Ok((api::ResultValue::Job(job), Some("job.changed")))
            }
            _ => unreachable!(),
        }
    }

    fn start_provider_inspection(
        &mut self,
        owner: usize,
        intent: ProviderInspectIntent,
    ) -> Result<(api::ResultValue, Option<&'static str>), Error> {
        self.app.plugin_foreground(&intent.context)?;
        if intent.context.terminal.is_some() {
            return Err(Error::new(
                Code::ContextChanged,
                "Remote inspection requires a visible provider document",
            ));
        }
        if self.app.host_buffer_is_closed(intent.buffer) {
            return Err(Error::new(Code::Closed, "Document closed"));
        }
        let buffer = &self.app.buffers[intent.buffer];
        if buffer.revision() != intent.expected_revision {
            return Err(Error::new(
                Code::Stale,
                "Document changed before inspection admission",
            ));
        }
        if buffer.len_bytes() > crate::diff_view::MAX_DIFF_BYTES {
            return Err(Error::new(
                Code::LimitExceeded,
                "Document exceeds the 4 MiB diff limit",
            ));
        }
        let identity = buffer
            .provider()
            .ok_or_else(|| Error::new(Code::Unsupported, "Document has no resource provider"))?
            .identity
            .clone();
        self.application_provider_read(
            owner,
            api::Request::ResourceOpen {
                plugin: identity.configured_plugin,
                provider: identity.provider,
                key: identity.key,
                invocation: None,
            },
            None,
            Some(intent),
        )
    }

    pub(super) fn sync_provider_inspections(&mut self) {
        for intent in self.app.take_provider_inspect_intents() {
            let owner = self
                .app
                .buffers
                .get(intent.buffer)
                .and_then(|b| b.provider())
                .and_then(|d| {
                    self.app
                        .plugins
                        .instances
                        .iter()
                        .find(|(_, i)| i.registered && i.config.id == d.identity.configured_plugin)
                        .map(|(owner, _)| *owner)
                });
            let result = owner
                .ok_or_else(|| Error::new(Code::Unavailable, "Resource provider is unavailable"))
                .and_then(|owner| {
                    self.start_provider_inspection(owner, intent)
                        .map(|(result, _)| (owner, result))
                });
            match result {
                Ok((owner, api::ResultValue::Job(job))) => {
                    if self
                        .application_job_event(owner, "job.changed", job)
                        .is_err()
                    {
                        self.stop_plugin(owner, "resource job consumer is too slow");
                    }
                }
                Ok(_) => unreachable!(),
                Err(error) => self.app.provider_save_feedback(false, &error.message),
            }
        }
    }

    fn send_provider_read(&mut self, job: &str) -> anyhow::Result<()> {
        let owner = self.provider_reads[job].owner;
        let state = &mut self
            .app
            .plugins
            .instances
            .get_mut(&owner)
            .unwrap()
            .application;
        anyhow::ensure!(
            state.requests.len() + state.provider_requests < api::MAX_REQUESTS,
            "Provider control request limit reached"
        );
        state.provider_requests += 1;
        self.app.plugins.next_invocation += 1;
        let id = format!("h:{}", self.app.plugins.next_invocation);
        let pending = self.provider_reads.get_mut(job).unwrap();
        pending.request = id.clone();
        let request = if let Some(metadata) = &pending.metadata {
            wire::Request::Read {
                job: job.into(),
                provider: pending.identity.provider.clone(),
                key: metadata.key.clone(),
                version: metadata.version.clone(),
                offset: pending.text.len(),
                limit: wire::CHUNK_BYTES,
            }
        } else if let Some(previous_write) = &pending.reconcile_write {
            wire::Request::Reconcile {
                job: job.into(),
                provider: pending.identity.provider.clone(),
                key: pending.identity.key.clone(),
                previous_write: previous_write.clone(),
            }
        } else {
            wire::Request::Stat {
                job: job.into(),
                provider: pending.identity.provider.clone(),
                key: pending.identity.key.clone(),
            }
        };
        let owner = pending.owner;
        self.plugin_send(
            owner,
            plugin::HostMessage::Deadline {
                token: id.clone(),
                after_ms: Some(10_000),
            },
        )?;
        self.application_send(owner, api::HostMessage::ResourceRequest { id, request })
    }

    pub(super) fn provider_response(
        &mut self,
        owner: usize,
        id: &str,
        outcome: &api::CommandResponse,
    ) -> anyhow::Result<bool> {
        let generation = &self.app.plugins.instances[&owner].application.generation;
        if self
            .provider_ignored
            .iter()
            .any(|(o, g, r)| *o == owner && g == generation && r == id)
        {
            return Ok(true);
        }
        let Some(job) = self
            .provider_reads
            .iter()
            .find(|(_, p)| p.owner == owner && &p.generation == generation && p.request == id)
            .map(|(job, _)| job.clone())
        else {
            return Ok(false);
        };
        self.plugin_send(
            owner,
            plugin::HostMessage::Deadline {
                token: id.into(),
                after_ms: None,
            },
        )?;
        self.app
            .plugins
            .instances
            .get_mut(&owner)
            .unwrap()
            .application
            .provider_requests -= 1;
        self.provider_reads.get_mut(&job).unwrap().request.clear();
        let result = match outcome {
            api::CommandResponse::Resource { result } => {
                self.accept_provider_read(&job, result.clone())
            }
            api::CommandResponse::Failure { error } if wire::safe_text(&error.message, 1024) => {
                Err(error.clone())
            }
            _ => Err(Error::new(
                Code::InvalidArgument,
                "Unexpected provider response",
            )),
        };
        match result {
            Ok(Some(index)) => self.finish_provider_read(&job, Ok(index)),
            Err(error) => self.finish_provider_read(&job, Err(error)),
            Ok(None) => self.send_provider_read(&job)?,
        }
        Ok(true)
    }

    fn accept_provider_read(
        &mut self,
        job: &str,
        response: wire::Response,
    ) -> Result<Option<usize>, Error> {
        let pending = &self.provider_reads[job];
        let response = if pending.metadata.is_none()
            && let Some(previous) = &pending.reconcile_write
        {
            match response {
                wire::Response::Reconciled {
                    metadata,
                    previous_write,
                } if &previous_write == previous => wire::Response::Stat(metadata),
                _ => {
                    return Err(Error::new(
                        Code::OutcomeUnknown,
                        "Provider must establish that the previous write is settled before reconciliation",
                    ));
                }
            }
        } else {
            response
        };
        if let wire::Response::Stat(metadata) = &response {
            let identity = &self.provider_reads[job].identity;
            if self.provider_reads.iter().any(|(other, p)| {
                other != job
                    && p.identity.configured_plugin == identity.configured_plugin
                    && p.identity.provider == identity.provider
                    && p.identity.key == metadata.key
            }) {
                return Err(Error::new(
                    Code::Busy,
                    "Canonical resource open is already pending",
                ));
            }
        }
        let pending = self.provider_reads.get_mut(job).unwrap();
        match (pending.metadata.as_ref(), response) {
            (None, wire::Response::Stat(metadata)) => {
                metadata.validate()?;
                if (pending.rebind.is_some() || pending.inspect.is_some())
                    && pending.identity.key != metadata.key
                {
                    return Err(Error::new(
                        Code::Conflict,
                        "Canonical resource identity changed during rebind",
                    ));
                }
                pending.identity.key = metadata.key.clone();
                if pending.rebind.is_none()
                    && pending.inspect.is_none()
                    && let Some((index, _)) =
                        self.app.buffers.iter().enumerate().find(|(index, buffer)| {
                            !self.app.host_buffer_is_closed(*index)
                                && buffer
                                    .provider()
                                    .is_some_and(|provider| provider.identity == pending.identity)
                        })
                {
                    return Ok(Some(index));
                }
                if pending.inspect.is_some() && metadata.bytes > crate::diff_view::MAX_DIFF_BYTES {
                    return Err(Error::new(
                        Code::LimitExceeded,
                        "Remote comparison exceeds the 4 MiB diff limit",
                    ));
                }
                pending.text = String::with_capacity(metadata.bytes);
                pending.metadata = Some(metadata);
                Ok(None)
            }
            (Some(metadata), wire::Response::Read(chunk)) => {
                if chunk.version != metadata.version {
                    return Err(Error::new(
                        Code::Stale,
                        "Resource version changed during read",
                    ));
                }
                if chunk.offset != pending.text.len()
                    || chunk.text.len() > wire::CHUNK_BYTES
                    || pending.text.len().saturating_add(chunk.text.len()) > metadata.bytes
                    || chunk.text.contains('\0')
                    || (!chunk.eof && chunk.text.is_empty())
                {
                    return Err(Error::new(Code::InvalidArgument, "Invalid resource chunk"));
                }
                pending.text.push_str(&chunk.text);
                if chunk.eof != (pending.text.len() == metadata.bytes) {
                    return Err(Error::new(
                        Code::InvalidArgument,
                        "Resource size and EOF disagree",
                    ));
                }
                if !chunk.eof {
                    return Ok(None);
                }
                if let Some(intent) = &pending.inspect {
                    let index = intent.buffer;
                    if self.app.host_buffer_is_closed(index) {
                        return Err(Error::new(
                            Code::Closed,
                            "Document closed during inspection",
                        ));
                    }
                    if !self.app.buffers[index].provider().is_some_and(|d| {
                        d.identity == pending.identity
                            && Some(d.baseline_epoch) == pending.rebind_epoch
                    }) {
                        return Err(Error::new(
                            Code::Conflict,
                            "Resource binding changed during inspection",
                        ));
                    }
                    let handles = &self.app.plugins.instances[&pending.requester]
                        .application
                        .buffers;
                    let existing = self.app.provider_inspection_snapshot(index);
                    if handles.len() >= 1024
                        && !existing.is_some_and(|snapshot| {
                            handles.values().any(|value| *value == snapshot)
                        })
                    {
                        return Err(Error::new(
                            Code::LimitExceeded,
                            "Buffer handle limit reached before publication",
                        ));
                    }
                    return self
                        .app
                        .publish_provider_inspection(
                            intent.clone(),
                            pending.generation.clone(),
                            metadata.version.clone(),
                            std::mem::take(&mut pending.text),
                        )
                        .map(Some)
                        .map_err(|error| Error::new(Code::ContextChanged, &error.to_string()));
                }
                if let Some(index) = pending.rebind {
                    if self.app.host_buffer_is_closed(index) {
                        return Err(Error::new(Code::Closed, "Document closed during rebind"));
                    }
                    if self.app.buffers[index].provider().map(|d| d.baseline_epoch)
                        != pending.rebind_epoch
                    {
                        return Err(Error::new(
                            Code::Conflict,
                            "Resource baseline changed during rebind",
                        ));
                    }
                    self.app.buffers[index].reconcile_provider(&pending.identity, pending.generation.clone(), metadata.version.clone(), &pending.text)
                        .map_err(|_| Error::new(Code::Conflict, "Remote text differs from accepted and uncertain baselines; inspect the conflict before overwriting"))?;
                    return Ok(Some(index));
                }
                let handles = &self.app.plugins.instances[&pending.requester]
                    .application
                    .buffers;
                let existing = self
                    .app
                    .buffers
                    .iter()
                    .enumerate()
                    .find(|(index, buffer)| {
                        !self.app.host_buffer_is_closed(*index)
                            && buffer
                                .provider()
                                .is_some_and(|p| p.identity == pending.identity)
                    })
                    .map(|(index, _)| index);
                if handles.len() >= 1024
                    && !existing.is_some_and(|index| handles.values().any(|value| *value == index))
                {
                    return Err(Error::new(
                        Code::LimitExceeded,
                        "Buffer handle limit reached before publication",
                    ));
                }
                let document = ProviderDocument {
                    identity: pending.identity.clone(),
                    label: metadata.label.clone(),
                    syntax_hint: metadata.syntax_hint.clone(),
                    version: metadata.version.clone(),
                    generation: pending.generation.clone(),
                    available: true,
                    baseline_epoch: 0,
                    uncertain: None,
                };
                // Publication can only reuse a live identity; a concurrent open never
                // replaces edits made after another request published that identity.
                let buffer = Buffer::provider_document(document, std::mem::take(&mut pending.text));
                Ok(Some(self.app.install_provider_document(buffer, false)))
            }
            _ => Err(Error::new(
                Code::InvalidArgument,
                "Unexpected resource response phase",
            )),
        }
    }

    fn finish_provider_read(&mut self, token: &str, result: Result<usize, Error>) {
        let Some(pending) = self.provider_reads.remove(token) else {
            return;
        };
        if let Some(index) = pending.rebind {
            self.app.plugins.document_saves.remove(&index);
        }
        self.reconcile_provider_payload();
        if !pending.request.is_empty() {
            if let Some(instance) = self
                .app
                .plugins
                .instances
                .get_mut(&pending.owner)
                .filter(|i| i.application.generation == pending.generation)
            {
                instance.application.provider_requests -= 1;
            }
            let _ = self.plugin_send(
                pending.owner,
                plugin::HostMessage::Deadline {
                    token: pending.request.clone(),
                    after_ms: None,
                },
            );
            self.provider_ignored
                .push_back((pending.owner, pending.generation, pending.request));
            while self.provider_ignored.len() > 64 {
                self.provider_ignored.pop_front();
            }
        }
        let Some(instance) = self
            .app
            .plugins
            .instances
            .get_mut(&pending.requester)
            .filter(|i| i.application.generation == pending.requester_generation)
        else {
            return;
        };
        instance.application.retained_payload -= pending.charge;
        let result = result.and_then(|index| {
            let handle = instance.application.buffer_handle(index)?;
            Ok((
                index,
                handle,
                format!("r:{}", self.app.buffers[index].revision()),
            ))
        });
        let job = instance.application.jobs.get_mut(token).unwrap();
        job.state = match &result {
            Ok(_) => api::JobState::Succeeded,
            Err(error) if error.code == Code::Cancelled => api::JobState::Cancelled,
            Err(_) => api::JobState::Failed,
        };
        if result.is_ok() {
            job.progress = 100;
        }
        let job = job.clone();
        instance.application.finished_jobs.push_back(token.into());
        if pending.inspect.is_some()
            && let Err(error) = &result
        {
            self.app.provider_save_feedback(false, &error.message);
        }
        let (buffer, revision, error) = match result {
            Ok((index, handle, revision)) => {
                if pending.inspect.is_none()
                    && let Some(context) = &pending.context
                    && self.app.plugin_foreground(context).is_ok()
                {
                    self.app.show_provider_document(index);
                }
                (Some(handle), Some(revision), None)
            }
            Err(error) => (None, None, Some(error)),
        };
        let state = &mut self
            .app
            .plugins
            .instances
            .get_mut(&pending.requester)
            .unwrap()
            .application;
        state.sequence += 1;
        let sequence = format!("e:{}", state.sequence);
        let finished = wire::Finished {
            job: token.into(),
            buffer,
            revision,
            error,
        };
        if self
            .plugin_send(
                pending.requester,
                plugin::HostMessage::Deadline {
                    token: token.into(),
                    after_ms: None,
                },
            )
            .is_err()
            || self
                .application_send(
                    pending.requester,
                    api::HostMessage::Event {
                        sequence,
                        event: if pending.inspect.is_some() {
                            "resource.inspected"
                        } else {
                            "resource.opened"
                        },
                        data: api::EventData::ResourceFinished(finished),
                    },
                )
                .is_err()
            || self
                .application_job_event(pending.requester, "job.changed", job)
                .is_err()
        {
            self.stop_plugin(pending.requester, "resource result consumer is too slow");
        }
        self.app.plugins.presentation_dirty = true;
    }

    pub(super) fn provider_job_request(
        &mut self,
        owner: usize,
        request: &api::Request,
    ) -> Option<Result<(api::ResultValue, Option<&'static str>), Error>> {
        let (api::Request::JobCancel { job }
        | api::Request::JobFinish { job, .. }
        | api::Request::JobUpdate { job, .. }) = request
        else {
            return None;
        };
        if !self
            .provider_reads
            .get(job)
            .is_some_and(|p| p.requester == owner)
        {
            return None;
        }
        if !matches!(request, api::Request::JobCancel { .. }) {
            return Some(Err(Error::new(
                Code::Conflict,
                "Provider operations are completed by the host",
            )));
        }
        self.finish_provider_read(
            job,
            Err(Error::new(Code::Cancelled, "Resource open cancelled")),
        );
        Some(
            self.app
                .plugins
                .instances
                .get(&owner)
                .and_then(|i| i.application.jobs.get(job))
                .cloned()
                .map(|job| (api::ResultValue::Job(job), None))
                .ok_or_else(|| Error::new(Code::Unavailable, "Application stopped")),
        )
    }

    pub(super) fn provider_deadline(&mut self, owner: usize, token: &str) -> bool {
        let job = self
            .provider_reads
            .iter()
            .find(|(job, p)| {
                (p.owner == owner && p.request == token)
                    || (p.requester == owner && job.as_str() == token)
            })
            .map(|(job, _)| job.clone());
        let Some(job) = job else { return false };
        self.finish_provider_read(
            &job,
            Err(Error::new(Code::Timeout, "Resource open timed out")),
        );
        true
    }

    pub(super) fn stop_provider_reads(&mut self, owner: usize) {
        // Remove before notifying peers so a slow peer's stop cannot recurse over
        // the same pending operation.
        let jobs: Vec<_> = self
            .provider_reads
            .iter()
            .filter(|(_, p)| p.owner == owner || p.requester == owner)
            .map(|(job, _)| job.clone())
            .collect();
        for job in jobs {
            self.finish_provider_read(
                &job,
                Err(Error::new(
                    Code::Unavailable,
                    "Resource provider or requester stopped",
                )),
            );
        }
        if let Some(instance) = self.app.plugins.instances.get(&owner) {
            let configured = &instance.config.id;
            for buffer in &mut self.app.buffers {
                if let Some(document) = buffer.provider_mut()
                    && &document.identity.configured_plugin == configured
                {
                    document.available = false;
                }
            }
        }
    }
}
