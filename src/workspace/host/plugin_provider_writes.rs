// SPDX-License-Identifier: MPL-2.0

use super::WorkspaceHost;
use crate::app::plugin_providers::ProviderSaveContinuation;
use crate::{
    buffer::ProviderSave,
    plugin::{self, application as api, provider as wire},
};
use api::{Error, ErrorCode as Code};

// Bounded trim transactions and their construction headroom for native previews.
const OVERWRITE_CHARGE: usize = wire::READ_CHARGE + 512 * 1024;

#[derive(Clone, Debug)]
enum Phase {
    AwaitApproval,
    Begin,
    Chunk { next: usize },
    Commit,
    Abort(Error),
}

pub(super) struct PendingWrite {
    pub requester: usize,
    pub requester_generation: String,
    pub owner: usize,
    pub generation: String,
    pub buffer: usize,
    pub orphaned: bool,
    charge_owner: String,
    charge: usize,
    snapshot: ProviderSave,
    preview: Option<crate::app::ProviderSavePreview>,
    mode: wire::WriteMode,
    atomic_replace: bool,
    request: String,
    phase: Phase,
    upload: Option<String>,
    offset: usize,
    commit_sent: bool,
    action: Option<u64>,
    continuation: Option<ProviderSaveContinuation>,
}

impl WorkspaceHost {
    pub(super) fn start_provider_write(
        &mut self,
        requester: usize,
        buffer: usize,
        continuation: Option<ProviderSaveContinuation>,
    ) -> Result<api::Job, Error> {
        self.start_provider_write_with_context(requester, buffer, continuation, None)
    }

    pub(super) fn start_provider_write_with_context(
        &mut self,
        requester: usize,
        buffer: usize,
        continuation: Option<ProviderSaveContinuation>,
        context: Option<api::CapturedContext>,
    ) -> Result<api::Job, Error> {
        let action = context.as_ref().and_then(|context| context.action);
        if self.app.host_buffer_is_closed(buffer) {
            return Err(Error::new(Code::Closed, "Document closed"));
        }
        if self.provider_writes.values().any(|p| p.buffer == buffer) {
            return Err(Error::new(Code::Busy, "Resource save is already pending"));
        }
        if self.app.buffers[buffer]
            .text()
            .rope()
            .chars()
            .any(|c| c == '\0')
        {
            return Err(Error::new(
                Code::InvalidArgument,
                "NUL is not supported in provider text",
            ));
        }
        let document = self.app.buffers[buffer]
            .provider()
            .ok_or_else(|| Error::new(Code::Unsupported, "Document has no resource provider"))?;
        if document.uncertain.is_some() {
            return Err(Error::new(
                Code::OutcomeUnknown,
                "Reconcile the previous remote write before retrying",
            ));
        }
        if !document.available {
            return Err(Error::new(
                Code::Unavailable,
                "Rebind the unavailable resource provider before saving",
            ));
        }
        if self.app.buffers[buffer].text().len_bytes() > wire::MAX_DOCUMENT_BYTES {
            return Err(Error::new(
                Code::LimitExceeded,
                "Resource exceeds document limit",
            ));
        }
        let (owner, instance) = self
            .app
            .plugins
            .instances
            .iter()
            .find(|(_, i)| {
                i.config.id == document.identity.configured_plugin
                    && i.application.generation == document.generation
                    && i.application
                        .providers
                        .contains_key(&document.identity.provider)
            })
            .ok_or_else(|| {
                Error::new(
                    Code::Unavailable,
                    "Resource provider binding is unavailable",
                )
            })?;
        let owner = *owner;
        let registration = &instance.application.providers[&document.identity.provider];
        let mode = if registration.conditional_write {
            wire::WriteMode::Conditional
        } else {
            let context = context.as_ref().ok_or_else(|| Error::new(Code::Unsupported,
                "Provider cannot conditionally overwrite; native foreground confirmation is required"))?;
            self.app.plugin_foreground(context)?;
            if context.buffer != buffer || context.terminal.is_some() {
                return Err(Error::new(
                    Code::ContextChanged,
                    "Overwrite requires the visible provider document",
                ));
            }
            wire::WriteMode::ConfirmedBestEffort
        };
        let atomic_replace = registration.atomic_replace;
        let label = document.label.clone();
        if instance.application.requests.len() + instance.application.provider_requests
            >= api::MAX_REQUESTS
            || self
                .provider_writes
                .values()
                .filter(|p| p.owner == owner)
                .count()
                >= 2
        {
            return Err(Error::new(Code::Busy, "Resource provider is busy"));
        }
        let generation = instance.application.generation.clone();
        let charge = if mode == wire::WriteMode::ConfirmedBestEffort {
            OVERWRITE_CHARGE
        } else {
            wire::READ_CHARGE
        };
        self.reserve_application_payload(requester, charge)?;
        let state = &self.app.plugins.instances[&requester].application;
        if state.buffers.len() >= 1024 && !state.buffers.values().any(|index| *index == buffer) {
            return Err(Error::new(
                Code::LimitExceeded,
                "Buffer handle limit reached",
            ));
        }
        // A host-owned job does not require the provider to grant itself the
        // plugin-facing job.create capability; API callers are checked separately.
        let job = self.create_provider_job(requester, "Save resource")?;
        let prepared = if mode == wire::WriteMode::ConfirmedBestEffort {
            self.app
                .preview_provider_save(buffer)
                .map(|preview| (preview.snapshot.clone(), Some(preview)))
        } else {
            self.app
                .prepare_provider_save_text(buffer)
                .map(|snapshot| (snapshot, None))
        };
        let (snapshot, preview) = match prepared {
            Ok(value) => value,
            Err(error) => {
                self.app
                    .plugins
                    .instances
                    .get_mut(&requester)
                    .unwrap()
                    .application
                    .jobs
                    .remove(&job.job);
                let _ = self.plugin_send(
                    requester,
                    plugin::HostMessage::Deadline {
                        token: job.job,
                        after_ms: None,
                    },
                );
                let code = if error.is::<crate::app::ProviderSavePreviewLimit>() {
                    Code::LimitExceeded
                } else {
                    Code::Conflict
                };
                return Err(Error::new(code, &error.to_string()));
            }
        };
        if let Some(preview) = &preview
            && let Err(error) = self.app.show_provider_overwrite(
                job.job.clone(),
                buffer,
                preview.source_revision,
                context.unwrap(),
                label,
                atomic_replace,
            )
        {
            self.app
                .plugins
                .instances
                .get_mut(&requester)
                .unwrap()
                .application
                .jobs
                .remove(&job.job);
            let _ = self.plugin_send(
                requester,
                plugin::HostMessage::Deadline {
                    token: job.job,
                    after_ms: None,
                },
            );
            return Err(Error::new(Code::ContextChanged, &error.to_string()));
        }
        let state = &mut self
            .app
            .plugins
            .instances
            .get_mut(&requester)
            .unwrap()
            .application;
        // Admission and handle publication share this host turn. Keep the target
        // issued throughout confirmation/upload so completion cannot lose it to
        // unrelated handle allocation while the operation is pending.
        state
            .buffer_handle(buffer)
            .expect("buffer capacity checked during admission");
        state.retained_payload += charge;
        let requester_generation = state.generation.clone();
        let charge_owner = self.app.plugins.instances[&requester].config.id.clone();
        self.app.plugins.document_saves.insert(buffer);
        self.provider_writes.insert(
            job.job.clone(),
            PendingWrite {
                requester,
                requester_generation,
                owner,
                generation,
                buffer,
                orphaned: false,
                charge_owner,
                charge,
                snapshot,
                preview,
                mode,
                atomic_replace,
                request: String::new(),
                phase: if mode == wire::WriteMode::ConfirmedBestEffort {
                    Phase::AwaitApproval
                } else {
                    Phase::Begin
                },
                upload: None,
                offset: 0,
                commit_sent: false,
                action,
                continuation,
            },
        );
        if mode == wire::WriteMode::ConfirmedBestEffort {
            return Ok(job);
        }
        if self.send_provider_write(&job.job).is_err() {
            let pending = self.provider_writes.remove(&job.job).unwrap();
            self.release_write_call(&pending);
            self.app.plugins.document_saves.remove(&buffer);
            let state = &mut self
                .app
                .plugins
                .instances
                .get_mut(&requester)
                .unwrap()
                .application;
            state.retained_payload -= pending.charge;
            state.jobs.remove(&job.job);
            let _ = self.plugin_send(
                requester,
                plugin::HostMessage::Deadline {
                    token: job.job,
                    after_ms: None,
                },
            );
            self.stop_plugin(owner, "resource write request consumer is too slow");
            return Err(Error::new(
                Code::Unavailable,
                "Provider queue is unavailable",
            ));
        }
        Ok(job)
    }

    pub(super) fn create_provider_job(
        &mut self,
        owner: usize,
        title: &str,
    ) -> Result<api::Job, Error> {
        let state = &mut self
            .app
            .plugins
            .instances
            .get_mut(&owner)
            .unwrap()
            .application;
        if state.jobs.values().filter(|job| job.state.active()).count() >= api::MAX_JOBS {
            return Err(Error::new(Code::LimitExceeded, "Finite job limit reached"));
        }
        if state.jobs.len() >= 64 {
            let oldest = state.finished_jobs.pop_front().unwrap();
            state.jobs.remove(&oldest);
            state.job_actions.remove(&oldest);
        }
        state.next_handle += 1;
        let job = api::Job {
            job: format!("j:{}:{}", state.generation, state.next_handle),
            title: title.into(),
            state: api::JobState::Running,
            progress: 0,
        };
        state.jobs.insert(job.job.clone(), job.clone());
        if self
            .plugin_send(
                owner,
                plugin::HostMessage::Deadline {
                    token: job.job.clone(),
                    after_ms: Some(60_000),
                },
            )
            .is_err()
        {
            self.stop_plugin(owner, "resource job admission failed");
            return Err(Error::new(
                Code::Unavailable,
                "Application queue unavailable",
            ));
        }
        Ok(job)
    }

    fn send_provider_write(&mut self, token: &str) -> anyhow::Result<()> {
        anyhow::ensure!(
            !matches!(self.provider_writes[token].phase, Phase::AwaitApproval),
            "Overwrite awaits native confirmation"
        );
        let owner = self.provider_writes[token].owner;
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
        let pending = self.provider_writes.get_mut(token).unwrap();
        pending.request = id.clone();
        let job = token.to_owned();
        let request = match &pending.phase {
            Phase::AwaitApproval => unreachable!("checked before reserving a call"),
            Phase::Begin => wire::Request::WriteBegin {
                job,
                provider: pending.snapshot.identity.provider.clone(),
                key: pending.snapshot.identity.key.clone(),
                expected_version: pending.snapshot.version.clone(),
                mode: pending.mode,
                bytes: pending.snapshot.text.len_bytes(),
                encoding: "utf-8",
            },
            Phase::Chunk { next } => {
                let rope = pending.snapshot.text.rope();
                let text = rope
                    .slice(rope.byte_to_char(pending.offset)..rope.byte_to_char(*next))
                    .to_string();
                wire::Request::WriteChunk {
                    job,
                    upload: pending.upload.clone().unwrap(),
                    offset: pending.offset,
                    text,
                }
            }
            Phase::Commit => wire::Request::WriteCommit {
                job,
                upload: pending.upload.clone().unwrap(),
                expected_version: pending.snapshot.version.clone(),
                mode: pending.mode,
            },
            Phase::Abort(_) => wire::Request::WriteAbort {
                job,
                upload: pending.upload.clone(),
            },
        };
        let committing = matches!(pending.phase, Phase::Commit);
        let timeout = if matches!(pending.phase, Phase::Abort(_)) {
            2000
        } else {
            10_000
        };
        self.plugin_send(
            owner,
            plugin::HostMessage::Deadline {
                token: id.clone(),
                after_ms: Some(timeout),
            },
        )?;
        self.application_send(owner, api::HostMessage::ResourceRequest { id, request })?;
        if committing {
            self.provider_writes.get_mut(token).unwrap().commit_sent = true;
        }
        Ok(())
    }

    fn advance_provider_write(&mut self, token: &str) -> anyhow::Result<()> {
        let pending = self.provider_writes.get_mut(token).unwrap();
        let rope = pending.snapshot.text.rope();
        pending.phase = if pending.offset == rope.len_bytes() {
            Phase::Commit
        } else {
            let end = pending
                .offset
                .saturating_add(wire::CHUNK_BYTES)
                .min(rope.len_bytes());
            let next = rope.char_to_byte(rope.byte_to_char(end));
            Phase::Chunk { next }
        };
        self.send_provider_write(token)
    }

    pub(super) fn provider_write_response(
        &mut self,
        owner: usize,
        id: &str,
        outcome: &api::CommandResponse,
    ) -> anyhow::Result<bool> {
        let generation = &self.app.plugins.instances[&owner].application.generation;
        let Some(token) = self
            .provider_writes
            .iter()
            .find(|(_, p)| {
                p.owner == owner
                    && &p.generation == generation
                    && !p.request.is_empty()
                    && p.request == id
            })
            .map(|(token, _)| token.clone())
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
        self.provider_writes
            .get_mut(&token)
            .unwrap()
            .request
            .clear();
        let pending = &self.provider_writes[&token];
        match (&pending.phase, outcome) {
            (
                Phase::Begin,
                api::CommandResponse::Resource {
                    result: wire::Response::WriteStarted { upload },
                },
            ) if wire::safe_text(upload, 256) => {
                self.provider_writes.get_mut(&token).unwrap().upload = Some(upload.clone());
                self.advance_provider_write(&token)?;
            }
            (
                Phase::Chunk { next },
                api::CommandResponse::Resource {
                    result: wire::Response::WriteChunk { offset },
                },
            ) if next == offset => {
                self.provider_writes.get_mut(&token).unwrap().offset = *offset;
                self.advance_provider_write(&token)?;
            }
            (
                Phase::Commit,
                api::CommandResponse::Resource {
                    result: wire::Response::WriteCommitted { version },
                },
            ) if wire::safe_text(version, 256) => {
                self.finish_provider_write(&token, Ok(version.clone()), false);
            }
            (
                Phase::Commit,
                api::CommandResponse::Resource {
                    result: wire::Response::WriteRejected { error },
                },
            ) if wire::safe_text(&error.message, 1024) => {
                self.finish_provider_write(
                    &token,
                    Err(error.clone()),
                    error.code == Code::OutcomeUnknown,
                );
            }
            (
                Phase::Abort(reason),
                api::CommandResponse::Resource {
                    result: wire::Response::WriteAborted {},
                },
            ) => {
                self.finish_provider_write(&token, Err(reason.clone()), false);
            }
            (_, api::CommandResponse::Failure { error })
                if wire::safe_text(&error.message, 1024) =>
            {
                let error = error.clone();
                if pending.commit_sent {
                    self.finish_provider_write(&token, Err(error), true);
                } else if matches!(pending.phase, Phase::Abort(_)) {
                    anyhow::bail!("provider could not abort staged write");
                } else {
                    self.cancel_provider_write(&token, error)?;
                }
            }
            _ => {
                if pending.commit_sent {
                    self.finish_provider_write(
                        &token,
                        Err(Error::new(
                            Code::OutcomeUnknown,
                            "Invalid remote commit acknowledgement",
                        )),
                        true,
                    );
                } else {
                    anyhow::bail!("invalid resource write response");
                }
            }
        }
        Ok(true)
    }

    fn release_write_call(&mut self, pending: &PendingWrite) {
        if pending.request.is_empty() {
            return;
        }
        if let Some(instance) = self
            .app
            .plugins
            .instances
            .get_mut(&pending.owner)
            .filter(|i| i.application.generation == pending.generation)
        {
            instance.application.provider_requests -= 1;
            let _ = self.plugin_send(
                pending.owner,
                plugin::HostMessage::Deadline {
                    token: pending.request.clone(),
                    after_ms: None,
                },
            );
        }
        self.provider_ignored.push_back((
            pending.owner,
            pending.generation.clone(),
            pending.request.clone(),
        ));
        while self.provider_ignored.len() > 64 {
            self.provider_ignored.pop_front();
        }
    }

    fn cancel_provider_write(&mut self, token: &str, reason: Error) -> anyhow::Result<()> {
        if matches!(self.provider_writes[token].phase, Phase::AwaitApproval) {
            self.finish_provider_write(token, Err(reason), false);
            return Ok(());
        }
        if self.provider_writes[token].commit_sent {
            self.finish_provider_write(token, Err(reason), true);
            return Ok(());
        }
        if matches!(self.provider_writes[token].phase, Phase::Abort(_)) {
            return Ok(());
        }
        let mut pending = self.provider_writes.remove(token).unwrap();
        self.release_write_call(&pending);
        pending.request.clear();
        pending.phase = Phase::Abort(reason);
        let _ = self.plugin_send(
            pending.requester,
            plugin::HostMessage::Deadline {
                token: token.into(),
                after_ms: None,
            },
        );
        self.provider_writes.insert(token.into(), pending);
        self.send_provider_write(token)
    }

    fn finish_provider_write(
        &mut self,
        token: &str,
        result: Result<String, Error>,
        mut unknown: bool,
    ) {
        let Some(pending) = self.provider_writes.remove(token) else {
            return;
        };
        self.app.clear_provider_overwrite(token);
        self.release_write_call(&pending);
        self.app.plugins.document_saves.remove(&pending.buffer);
        let live = self
            .app
            .plugins
            .instances
            .get(&pending.requester)
            .is_some_and(|i| i.application.generation == pending.requester_generation);
        if pending.orphaned {
            self.app.plugins.orphaned_payload -= pending.charge;
        } else if live {
            self.app
                .plugins
                .instances
                .get_mut(&pending.requester)
                .unwrap()
                .application
                .retained_payload -= pending.charge;
        }
        let accepted = if let Ok(version) = &result {
            let accepted = !self.app.host_buffer_is_closed(pending.buffer)
                && self.app.buffers[pending.buffer]
                    .accept_provider_save(pending.snapshot.clone(), version.clone());
            unknown |= !accepted;
            accepted
        } else {
            false
        };
        let error = if unknown {
            if self.app.buffers[pending.buffer].mark_provider_uncertain(pending.snapshot.clone()) {
                self.app.plugins.orphaned_payload += wire::READ_CHARGE;
                self.provider_uncertain.insert(
                    pending.buffer,
                    (
                        pending.charge_owner.clone(),
                        wire::READ_CHARGE,
                        token.into(),
                    ),
                );
            }
            Some(Error::new(
                Code::OutcomeUnknown,
                "Remote write may have committed; explicitly rebind and reconcile before retrying",
            ))
        } else {
            result.err()
        };
        let terminal = if unknown {
            api::JobState::OutcomeUnknown
        } else if accepted {
            api::JobState::Succeeded
        } else if error
            .as_ref()
            .is_some_and(|error| error.code == Code::Cancelled)
        {
            api::JobState::Cancelled
        } else {
            api::JobState::Failed
        };
        if let Some(error) = &error {
            self.app
                .provider_save_action_feedback(pending.action, false, &error.message);
        } else {
            self.app
                .provider_save_action_feedback(pending.action, true, "Remote document saved");
        }
        if live && !pending.orphaned {
            let state = &mut self
                .app
                .plugins
                .instances
                .get_mut(&pending.requester)
                .unwrap()
                .application;
            let job = state.jobs.get_mut(token).unwrap();
            job.state = terminal;
            if accepted {
                job.progress = 100;
            }
            let job = job.clone();
            state.finished_jobs.push_back(token.into());
            let handle = state.buffer_handle(pending.buffer).ok();
            state.sequence += 1;
            let sequence = format!("e:{}", state.sequence);
            let finished = wire::Finished {
                job: token.into(),
                buffer: handle,
                revision: accepted.then(|| format!("r:{}", pending.snapshot.text.revision())),
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
                            event: "resource.saved",
                            data: api::EventData::ResourceFinished(finished),
                        },
                    )
                    .is_err()
                || self
                    .application_job_event(pending.requester, "job.changed", job)
                    .is_err()
            {
                self.stop_plugin(
                    pending.requester,
                    "resource save result consumer is too slow",
                );
            }
        }
        if accepted && !unknown {
            self.app
                .finish_provider_save_continuation(pending.continuation);
        }
        self.reconcile_wait_requests();
        self.app.plugins.presentation_dirty = true;
    }

    pub(super) fn provider_write_job_request(
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
            .provider_writes
            .get(job)
            .is_some_and(|p| p.requester == owner)
        {
            return None;
        }
        if !matches!(request, api::Request::JobCancel { .. }) {
            return Some(Err(Error::new(
                Code::Conflict,
                "Provider writes are completed by the host",
            )));
        }
        let provider = self.provider_writes[job].owner;
        if self
            .cancel_provider_write(job, Error::new(Code::Cancelled, "Resource save cancelled"))
            .is_err()
        {
            self.stop_plugin(provider, "provider cancellation queue unavailable");
        }
        if let Some(state) = self
            .app
            .plugins
            .instances
            .get_mut(&owner)
            .map(|i| &mut i.application)
            && let Some(job) = state.jobs.get_mut(job)
        {
            let event = (job.state == api::JobState::Running).then_some("job.changed");
            if job.state.active() {
                job.state = api::JobState::Cancelling;
            }
            return Some(Ok((api::ResultValue::Job(job.clone()), event)));
        }
        Some(Err(Error::new(Code::Unavailable, "Application stopped")))
    }

    pub(super) fn provider_write_deadline(
        &mut self,
        owner: usize,
        token: &str,
    ) -> anyhow::Result<bool> {
        let Some(job) = self
            .provider_writes
            .iter()
            .find(|(job, p)| {
                (p.owner == owner && p.request == token)
                    || (p.requester == owner && job.as_str() == token)
            })
            .map(|(job, _)| job.clone())
        else {
            return Ok(false);
        };
        let pending = &self.provider_writes[&job];
        let provider = pending.owner;
        if matches!(pending.phase, Phase::Abort(_)) {
            // A queued original job deadline cannot shorten the abort deadline.
            if token != job {
                self.stop_plugin(provider, "provider did not acknowledge write abort");
            }
            return Ok(true);
        }
        if self
            .cancel_provider_write(&job, Error::new(Code::Timeout, "Resource save timed out"))
            .is_err()
        {
            self.stop_plugin(provider, "provider cancellation queue unavailable");
        }
        Ok(true)
    }

    pub(super) fn stop_provider_writes(&mut self, owner: usize) {
        let jobs: Vec<_> = self
            .provider_writes
            .iter()
            .filter(|(_, p)| p.owner == owner || p.requester == owner)
            .map(|(job, _)| job.clone())
            .collect();
        for job in jobs {
            let Some(pending) = self.provider_writes.get(&job) else {
                continue;
            };
            if pending.owner == owner || matches!(pending.phase, Phase::AwaitApproval) {
                let unknown = pending.commit_sent;
                self.finish_provider_write(
                    &job,
                    Err(Error::new(Code::Unavailable, "Resource provider stopped")),
                    unknown,
                );
            } else {
                let provider = pending.owner;
                let pending = self.provider_writes.get_mut(&job).unwrap();
                if !pending.orphaned {
                    pending.orphaned = true;
                    self.app.plugins.orphaned_payload += pending.charge;
                }
                pending.continuation = None;
                if self
                    .cancel_provider_write(
                        &job,
                        Error::new(Code::Cancelled, "Requesting application stopped"),
                    )
                    .is_err()
                {
                    self.stop_plugin(provider, "provider cancellation queue unavailable");
                }
            }
        }
    }

    fn provider_overwrite_valid(&self, token: &str) -> bool {
        let pending = &self.provider_writes[token];
        let Some(preview) = &pending.preview else {
            return false;
        };
        if self.app.host_buffer_is_closed(pending.buffer)
            || self.app.buffers[pending.buffer].revision() != preview.source_revision
        {
            return false;
        }
        // Unchanged text was validated at admission. Pending-overlay checks must
        // stay constant-time rather than rescan an 8 MiB document on each input.
        let Some(current) = self.app.buffers[pending.buffer].provider() else {
            return false;
        };
        let snapshot = &pending.snapshot;
        if !current.available
            || current.uncertain.is_some()
            || current.identity != snapshot.identity
            || current.generation != snapshot.generation
            || current.baseline_epoch != snapshot.epoch
            || current.version != snapshot.version
        {
            return false;
        }
        self.app
            .plugins
            .instances
            .get(&pending.owner)
            .is_some_and(|instance| {
                instance.registered
                    && instance.application.generation == pending.generation
                    && instance
                        .application
                        .providers
                        .get(&snapshot.identity.provider)
                        .is_some_and(|registration| {
                            !registration.conditional_write
                                && registration.atomic_replace == pending.atomic_replace
                        })
            })
    }

    fn sync_provider_overwrite_approvals(&mut self) {
        self.app.sync_provider_overwrite();
        for decision in self.app.take_provider_overwrite_decisions() {
            let token = decision.job;
            if !self
                .provider_writes
                .get(&token)
                .is_some_and(|p| matches!(p.phase, Phase::AwaitApproval))
            {
                continue;
            }
            let Some(context) = decision.context else {
                self.finish_provider_write(
                    &token,
                    Err(Error::new(Code::Cancelled, "Remote overwrite cancelled")),
                    false,
                );
                continue;
            };
            if self.app.plugin_foreground(&context).is_err()
                || !self.provider_overwrite_valid(&token)
            {
                self.finish_provider_write(
                    &token,
                    Err(Error::new(
                        Code::ContextChanged,
                        "Overwrite context or resource changed; review the save again",
                    )),
                    false,
                );
                continue;
            }
            let pending = &self.provider_writes[&token];
            if context.buffer != pending.buffer || context.terminal.is_some() {
                self.finish_provider_write(
                    &token,
                    Err(Error::new(Code::ContextChanged, "Overwrite target changed")),
                    false,
                );
                continue;
            }
            let state = &self.app.plugins.instances[&pending.owner].application;
            if state.requests.len() + state.provider_requests >= api::MAX_REQUESTS {
                // A busy retry remains a visible decision requiring another Enter.
                let result = self.app.show_provider_overwrite(
                    token.clone(),
                    pending.buffer,
                    pending.preview.as_ref().unwrap().source_revision,
                    context,
                    self.app.buffers[pending.buffer]
                        .provider()
                        .unwrap()
                        .label
                        .clone(),
                    pending.atomic_replace,
                );
                if result.is_err() {
                    self.finish_provider_write(
                        &token,
                        Err(Error::new(Code::Busy, "Provider is busy; retry the save")),
                        false,
                    );
                }
                continue;
            }
            let pending = self.provider_writes.get_mut(&token).unwrap();
            let preview = pending.preview.take().unwrap();
            let prepared = self
                .app
                .apply_provider_save_preview(pending.buffer, preview);
            match prepared {
                Ok(snapshot) => {
                    pending.snapshot = snapshot;
                    pending.continuation = self
                        .app
                        .refresh_provider_save_continuation(pending.continuation, &context);
                    pending.phase = Phase::Begin;
                    let owner = pending.owner;
                    if self.send_provider_write(&token).is_err() {
                        self.stop_plugin(owner, "Provider overwrite queue unavailable");
                    }
                }
                Err(_) => self.finish_provider_write(
                    &token,
                    Err(Error::new(
                        Code::Conflict,
                        "Resource changed before overwrite acceptance",
                    )),
                    false,
                ),
            }
        }
        let stale: Vec<_> = self
            .provider_writes
            .iter()
            .filter(|(token, pending)| {
                matches!(pending.phase, Phase::AwaitApproval)
                    && !self.provider_overwrite_valid(token)
            })
            .map(|(token, _)| token.clone())
            .collect();
        for token in stale {
            self.finish_provider_write(
                &token,
                Err(Error::new(
                    Code::ContextChanged,
                    "Resource changed while awaiting overwrite confirmation",
                )),
                false,
            );
        }
    }

    pub(super) fn sync_provider_writes(&mut self) {
        self.sync_provider_overwrite_approvals();
        for intent in self.app.take_provider_save_intents() {
            let action = intent.context.action;
            if self.app.buffers[intent.buffer].revision() != intent.expected_revision {
                self.app.plugins.document_saves.remove(&intent.buffer);
                self.app.provider_save_action_feedback(
                    action,
                    false,
                    "Document changed before save admission; retry the save",
                );
                continue;
            }
            let provider = self
                .app
                .buffers
                .get(intent.buffer)
                .and_then(|b| b.provider())
                .and_then(|d| {
                    self.app
                        .plugins
                        .instances
                        .iter()
                        .find(|(_, i)| i.config.id == d.identity.configured_plugin)
                        .map(|(owner, _)| *owner)
                });
            let result = provider
                .ok_or_else(|| Error::new(Code::Unavailable, "Resource provider is unavailable"))
                .and_then(|owner| {
                    self.start_provider_write_with_context(
                        owner,
                        intent.buffer,
                        intent.continuation,
                        Some(intent.context),
                    )
                    .map(|job| (owner, job))
                });
            match result {
                Ok((owner, job)) => {
                    if self
                        .application_job_event(owner, "job.changed", job)
                        .is_err()
                    {
                        self.stop_plugin(owner, "resource job consumer is too slow");
                    }
                }
                Err(error) => {
                    self.app.plugins.document_saves.remove(&intent.buffer);
                    self.app
                        .provider_save_action_feedback(action, false, &error.message);
                }
            }
        }
        self.reconcile_provider_payload();
    }

    pub(super) fn reconcile_provider_payload(&mut self) {
        let released: Vec<_> = self
            .provider_uncertain
            .keys()
            .copied()
            .filter(|index| {
                !self
                    .plugin_recoveries
                    .values()
                    .any(|pending| pending.retains_uncertainty(*index))
                    && (self.app.host_buffer_is_closed(*index)
                        || self.app.buffers[*index]
                            .provider()
                            .is_none_or(|d| d.uncertain.is_none()))
            })
            .collect();
        for index in released {
            if self.app.host_buffer_is_closed(index)
                && let Some(document) = self.app.buffers[index].provider_mut()
            {
                document.uncertain = None;
            }
            let (_, charge, _) = self.provider_uncertain.remove(&index).unwrap();
            self.app.plugins.orphaned_payload -= charge;
        }
    }
}
