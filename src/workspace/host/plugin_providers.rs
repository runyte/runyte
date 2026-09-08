// SPDX-License-Identifier: MPL-2.0

use super::WorkspaceHost;
use crate::{
    buffer::{Buffer, ProviderDocument, ProviderIdentity},
    plugin::{self, application as api, provider as wire},
};
use api::{Error, ErrorCode as Code};

pub(super) struct PendingRead {
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
        let state = &self.app.plugins.instances[&owner].application;
        match request {
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
                if !state.capabilities.contains("documents") || !state.capabilities.contains("jobs")
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
                self.reserve_application_payload(owner, wire::READ_CHARGE)?;
                let identity = ProviderIdentity {
                    configured_plugin: plugin,
                    provider,
                    key,
                };
                if self.provider_reads.values().any(|p| p.identity == identity) {
                    return Err(Error::new(Code::Busy, "Resource open is already pending"));
                }
                let created = self.application_request(
                    owner,
                    api::Request::JobCreate {
                        title: "Open resource".into(),
                        deadline_seconds: 60,
                    },
                );
                let (api::ResultValue::Job(job), _) = (match created {
                    Ok(result) => result,
                    Err(error) => {
                        if error.code == Code::Unavailable {
                            self.stop_plugin(owner, "resource job admission failed");
                        }
                        return Err(error);
                    }
                }) else {
                    unreachable!()
                };
                self.app
                    .plugins
                    .instances
                    .get_mut(&owner)
                    .unwrap()
                    .application
                    .retained_payload += wire::READ_CHARGE;
                self.provider_reads.insert(
                    job.job.clone(),
                    PendingRead {
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
                    state.retained_payload -= wire::READ_CHARGE;
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
                pending.identity.key = metadata.key.clone();
                if let Some((index, _)) =
                    self.app.buffers.iter().enumerate().find(|(index, buffer)| {
                        !self.app.host_buffer_is_closed(*index)
                            && buffer
                                .provider()
                                .is_some_and(|provider| provider.identity == pending.identity)
                    })
                {
                    return Ok(Some(index));
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
        instance.application.retained_payload -= wire::READ_CHARGE;
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
        let (buffer, revision, error) = match result {
            Ok((index, handle, revision)) => {
                if let Some(context) = &pending.context
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
                        event: "resource.opened",
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
