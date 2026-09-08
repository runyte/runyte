// SPDX-License-Identifier: MPL-2.0

use super::WorkspaceHost;
use crate::plugin::{self, application as api};
use anyhow::{Result, ensure};
use std::collections::BTreeSet;

impl WorkspaceHost {
    pub(super) fn application_message(
        &mut self,
        id: usize,
        message: api::ClientMessage,
    ) -> Result<()> {
        match message {
            api::ClientMessage::Register {
                version,
                name,
                commands,
                required_capabilities,
                optional_capabilities,
            } => {
                ensure!(version == api::VERSION, "unsupported application epoch");
                ensure!(
                    !self.app.plugins.instances[&id].registered,
                    "application already registered"
                );
                ensure!(safe_label(&name, 80), "invalid application name");
                ensure!(
                    required_capabilities.len() + optional_capabilities.len() <= 32,
                    "too many capabilities"
                );
                let configured = &self.app.plugins.instances[&id].config.capabilities;
                let capabilities = required_capabilities
                    .union(&optional_capabilities)
                    .filter(|cap| {
                        api::CAPABILITIES.contains(&cap.as_str()) && configured.contains(cap)
                    })
                    .cloned()
                    .collect::<BTreeSet<_>>();
                ensure!(
                    required_capabilities.is_subset(&capabilities),
                    "required application capability unavailable"
                );
                ensure!(
                    commands.iter().filter(|c| c.primary).count() <= 1
                        && commands
                            .iter()
                            .all(|c| !c.primary || c.context == api::CommandContext::View),
                    "invalid primary view action"
                );
                for command in &commands {
                    crate::plugin::arguments::validate(&command.arguments)
                        .map_err(|error| anyhow::anyhow!(error.message))?;
                }
                let arguments = commands
                    .iter()
                    .map(|command| (command.name.clone(), command.arguments.clone()))
                    .collect();
                let primaries = commands
                    .iter()
                    .filter(|c| c.primary)
                    .map(|c| c.name.clone())
                    .collect();
                // The existing atomic command/keymap installer remains the only registry.
                let contexts = commands
                    .iter()
                    .map(|c| (c.name.clone(), c.context))
                    .collect::<std::collections::BTreeMap<_, _>>();
                self.app
                    .plugins
                    .instances
                    .get_mut(&id)
                    .unwrap()
                    .application
                    .capabilities = capabilities.clone();
                self.app
                    .plugins
                    .instances
                    .get_mut(&id)
                    .unwrap()
                    .application
                    .command_contexts = contexts.clone();
                self.app
                    .plugins
                    .instances
                    .get_mut(&id)
                    .unwrap()
                    .application
                    .primary_commands = primaries;
                self.app
                    .plugins
                    .instances
                    .get_mut(&id)
                    .unwrap()
                    .application
                    .command_arguments = arguments;
                self.plugin_message(
                    id,
                    plugin::ClientMessage::Register {
                        version: plugin::VERSION.into(),
                        commands: commands
                            .into_iter()
                            .map(|c| plugin::Registration {
                                name: c.name,
                                description: c.description,
                            })
                            .collect(),
                    },
                )?;
                for command in self
                    .app
                    .plugins
                    .commands
                    .values_mut()
                    .filter(|c| c.plugin == id)
                {
                    if let Some(context) = contexts.get(&command.local) {
                        command.context = *context;
                    }
                }
                self.app
                    .plugins
                    .instances
                    .get_mut(&id)
                    .unwrap()
                    .application
                    .capabilities = capabilities;
            }
            api::ClientMessage::Request {
                id: request_id,
                request,
            } => {
                self.application_request_id(id, &request_id)?;
                if matches!(
                    request,
                    api::Request::FilesystemList { .. }
                        | api::Request::FilesystemPrepare { .. }
                        | api::Request::FilesystemApply { .. }
                        | api::Request::FilesystemCancel { .. }
                        | api::Request::FilesystemRelease { .. }
                        | api::Request::BufferOpen { .. }
                        | api::Request::BufferCreate { .. }
                ) {
                    let result = self.application_filesystem_request(id, &request_id, request);
                    return match result {
                        Ok(None) => Ok(()),
                        Ok(Some(result)) => {
                            self.application_local_reply(id, request_id, Ok(result))
                        }
                        Err(error) => self.application_local_reply(id, request_id, Err(error)),
                    };
                }
                if matches!(
                    request,
                    api::Request::UiForm { .. }
                        | api::Request::UiPrompt { .. }
                        | api::Request::UiPick { .. }
                        | api::Request::UiConfirm { .. }
                        | api::Request::UiDismiss { .. }
                ) {
                    let result = self.application_input_request(id, request);
                    return self.application_local_reply(id, request_id, result);
                }
                if matches!(
                    request,
                    api::Request::BufferSave { .. } | api::Request::BufferClose { .. }
                ) {
                    return self.application_document_request(id, request_id, request);
                }
                let result = self.application_request(id, request);
                let event = result
                    .as_ref()
                    .ok()
                    .and_then(|(result, event)| match result {
                        api::ResultValue::Job(job) => event.map(|event| (event, job.clone())),
                        _ => None,
                    });
                let outcome = match result {
                    Ok((result, _)) => api::Response::Success { result },
                    Err(error) => api::Response::Failure { error },
                };
                self.application_send(
                    id,
                    api::HostMessage::Response {
                        id: request_id,
                        outcome,
                    },
                )?;
                if let Some((event, job)) = event {
                    self.application_job_event(id, event, job)?;
                }
            }
            api::ClientMessage::Response {
                id: request_id,
                outcome,
            } => {
                let instance = self.app.plugins.instances.get_mut(&id).unwrap();
                ensure!(instance.registered, "application must register first");
                let Some(action) = instance.application.requests.remove(&request_id) else {
                    anyhow::bail!("unknown or completed application request");
                };
                if let api::CommandResponse::Success { result } = &outcome
                    && let Some(job) = &result.job
                {
                    ensure!(
                        instance.application.jobs.contains_key(job),
                        "unowned accepted job"
                    );
                }
                if let api::CommandResponse::Success { result } = &outcome {
                    let detail = if let Some(job) = &result.job {
                        instance
                            .application
                            .job_actions
                            .insert(job.clone(), action.action);
                        if instance.application.jobs[job].state.active() {
                            "Background job accepted"
                        } else {
                            "Background job finished"
                        }
                    } else {
                        "Application command completed"
                    };
                    self.app.plugin_application_feedback(action.action, detail);
                }
                self.plugin_send(
                    id,
                    plugin::HostMessage::Deadline {
                        token: request_id,
                        after_ms: None,
                    },
                )?;
                if let api::CommandResponse::Failure { error } = outcome {
                    ensure!(
                        safe_label(&error.message, 1024),
                        "invalid application error message"
                    );
                    self.app.plugin_completion_feedback(
                        action.action,
                        "command",
                        "failed",
                        &error.message,
                    );
                }
            }
        }
        Ok(())
    }

    pub(super) fn application_request_id(&mut self, id: usize, request: &str) -> Result<()> {
        let instance = self.app.plugins.instances.get_mut(&id).unwrap();
        ensure!(instance.registered, "application must register first");
        let number = request
            .strip_prefix("p:")
            .and_then(|n| n.parse::<u64>().ok())
            .filter(|n| request == format!("p:{n}"));
        ensure!(
            number.is_some_and(|n| n > instance.application.last_request),
            "request ID must increase"
        );
        instance.application.last_request = number.unwrap();
        Ok(())
    }

    pub(super) fn application_send(&mut self, id: usize, message: api::HostMessage) -> Result<()> {
        self.plugin_send(id, plugin::HostMessage::Application(message))
    }

    pub(super) fn application_request(
        &mut self,
        id: usize,
        request: api::Request,
    ) -> Result<(api::ResultValue, Option<&'static str>), api::Error> {
        use api::{ErrorCode as Code, Request};
        let host_cancel = match &request {
            Request::JobGet { job }
            | Request::JobCancel { job }
            | Request::JobFinish { job, .. }
            | Request::JobUpdate { job, .. } => self.host_job_cancellation(id, job),
            _ => None,
        };
        if matches!(
            request,
            Request::JobFinish { .. } | Request::JobUpdate { .. }
        ) && host_cancel.is_some()
        {
            return Err(api::Error::new(
                Code::Conflict,
                "Host-owned jobs finish through IO completion",
            ));
        }
        if matches!(
            request,
            Request::BufferList { .. }
                | Request::PaneList(_)
                | Request::BufferRead { .. }
                | Request::BufferEdit { .. }
                | Request::SnapshotOpen { .. }
                | Request::SnapshotRead { .. }
                | Request::SnapshotClose { .. }
                | Request::SelectionGet { .. }
                | Request::SelectionSet { .. }
        ) {
            return self
                .application_editor_request(id, request)
                .map(|value| (value, None));
        }
        if matches!(
            request,
            Request::ViewCreate { .. }
                | Request::ViewGet { .. }
                | Request::ViewPublish { .. }
                | Request::ViewClose { .. }
                | Request::PaneShow { .. }
        ) {
            return self
                .application_view_request(id, request)
                .map(|value| (value, None));
        }

        let fail = api::Error::new;
        let capability = match request {
            Request::WorkspaceInfo(_) => "workspace",
            _ => "jobs",
        };
        if !self.app.plugins.instances[&id]
            .application
            .capabilities
            .contains(capability)
        {
            return Err(fail(Code::CapabilityDenied, "Capability was not granted"));
        }
        if let Request::JobCancel { job } = &request {
            self.cancel_queued_filesystem_job(id, job)?;
        }
        if matches!(request, Request::WorkspaceInfo(_)) {
            return Ok((
                api::ResultValue::Workspace {
                    name: self
                        .app
                        .project_root
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned(),
                },
                None,
            ));
        }
        let state = &mut self.app.plugins.instances.get_mut(&id).unwrap().application;
        let mut deadline = None;
        let mut event = Some("job.changed");
        let job = match request {
            Request::JobCreate {
                title,
                deadline_seconds,
            } => {
                if !safe_label(&title, 160) || !(1..=3600).contains(&deadline_seconds) {
                    return Err(fail(Code::InvalidArgument, "Invalid job title or deadline"));
                }
                if state.jobs.values().filter(|j| j.state.active()).count() >= api::MAX_JOBS {
                    return Err(fail(Code::LimitExceeded, "Finite job limit reached"));
                }
                // Retain a bounded recent terminal history for job.get.
                if state.jobs.len() >= 64 {
                    let oldest = state.finished_jobs.pop_front().unwrap();
                    state.jobs.remove(&oldest);
                    state.job_actions.remove(&oldest);
                }
                state.next_handle += 1;
                let handle = format!("j:{}:{}", state.generation, state.next_handle);
                let job = api::Job {
                    job: handle.clone(),
                    title,
                    state: api::JobState::Running,
                    progress: 0,
                };
                state.jobs.insert(handle.clone(), job.clone());
                deadline = Some((handle, Some(deadline_seconds * 1000)));
                job
            }
            ref operation => {
                let (Request::JobGet { job }
                | Request::JobUpdate { job, .. }
                | Request::JobFinish { job, .. }
                | Request::JobCancel { job }) = operation
                else {
                    unreachable!()
                };
                let job = state
                    .jobs
                    .get_mut(job)
                    .ok_or_else(|| fail(Code::NotFound, "Unknown job"))?;
                match operation {
                    Request::JobGet { .. } => event = None,
                    Request::JobUpdate { progress, .. } => {
                        if *progress > 100 {
                            return Err(fail(
                                Code::InvalidArgument,
                                "Progress must be between 0 and 100",
                            ));
                        }
                        if job.state != api::JobState::Running {
                            return Err(fail(Code::Conflict, "Job is no longer running"));
                        }
                        job.progress = *progress;
                        // Progress is recovered through job.get; it does not flood reliable delivery.
                        event = None;
                    }
                    Request::JobFinish { state, .. } => {
                        if !job.state.active() {
                            return Err(fail(Code::Conflict, "Job already finished"));
                        }
                        if job.state == api::JobState::Cancelling
                            && *state == api::TerminalState::Succeeded
                        {
                            return Err(fail(Code::Cancelled, "Cancellation already accepted"));
                        }
                        job.state = (*state).into();
                        deadline = Some((job.job.clone(), None));
                    }
                    Request::JobCancel { .. } => {
                        if job.state != api::JobState::Running {
                            event = None;
                        } else {
                            job.state = api::JobState::Cancelling;
                            if let Some(cancelled) = &host_cancel {
                                cancelled.store(true, std::sync::atomic::Ordering::SeqCst);
                                deadline = Some((job.job.clone(), None));
                                event = Some("job.changed");
                            } else {
                                deadline = Some((job.job.clone(), Some(2000)));
                                event = Some("job.cancel_requested");
                            }
                        }
                    }
                    _ => unreachable!(),
                }
                job.clone()
            }
        };
        if event.is_some() && !job.state.active() {
            state.finished_jobs.push_back(job.job.clone());
        }
        if let Some((token, after_ms)) = deadline {
            self.plugin_send(id, plugin::HostMessage::Deadline { token, after_ms })
                .map_err(|_| fail(Code::Unavailable, "Application queue unavailable"))?;
        }
        Ok((api::ResultValue::Job(job), event))
    }

    pub(super) fn application_deadline(&mut self, id: usize, token: String) -> Result<()> {
        let host_cancel = self.host_job_cancellation(id, &token);
        let filesystem_running = self
            .filesystem_apply
            .as_ref()
            .is_some_and(|p| p.owner == id && p.job == token);
        let instance = self.app.plugins.instances.get_mut(&id).unwrap();
        if instance
            .application
            .deadlines
            .get(&token)
            .is_some_and(|due| *due > std::time::Instant::now())
        {
            return Ok(());
        }
        instance.application.deadlines.remove(&token);
        if filesystem_running && self.cancel_queued_filesystem_job(id, &token).is_err() {
            // OS mutations cannot be interrupted or rolled back by a control deadline.
            return Ok(());
        }
        let instance = self.app.plugins.instances.get_mut(&id).unwrap();
        if let Some(cancelled) = host_cancel {
            cancelled.store(true, std::sync::atomic::Ordering::SeqCst);
            let changed = instance.application.jobs.get_mut(&token).and_then(|job| {
                if job.state != api::JobState::Running {
                    return None;
                }
                job.state = api::JobState::Cancelling;
                Some(job.clone())
            });
            if let Some(job) = changed {
                self.application_job_event(id, "job.changed", job)?;
                self.plugin_send(
                    id,
                    plugin::HostMessage::Deadline {
                        token,
                        after_ms: None,
                    },
                )?;
            }
            // Keep the job, snapshot and buffer guard until actual completion;
            // repeated queued deadlines cannot punish the plugin for host IO.
            return Ok(());
        }
        if let Some(snapshot) = instance.application.snapshots.remove(&token) {
            instance.application.retained_payload -= snapshot.text.len_bytes();
            return Ok(());
        }
        if instance.application.requests.contains_key(&token) {
            anyhow::bail!("application control request timed out");
        }
        let Some(job) = instance.application.jobs.get_mut(&token) else {
            return Ok(());
        };
        if job.state == api::JobState::Cancelling {
            anyhow::bail!("application did not acknowledge cancellation");
        }
        if !job.state.active() {
            return Ok(());
        }
        job.state = api::JobState::Cancelling;
        let job = job.clone();
        self.application_job_event(id, "job.cancel_requested", job)?;
        self.plugin_send(
            id,
            plugin::HostMessage::Deadline {
                token,
                after_ms: Some(2000),
            },
        )
    }

    pub(super) fn application_job_event(
        &mut self,
        id: usize,
        event: &'static str,
        job: api::Job,
    ) -> Result<()> {
        if !job.state.active() {
            let action = self.app.plugins.instances[&id]
                .application
                .job_actions
                .get(&job.job)
                .copied()
                .flatten();
            self.app
                .plugin_application_feedback(action, "Background job finished");
        }
        let state = &mut self.app.plugins.instances.get_mut(&id).unwrap().application;
        state.sequence += 1;
        let sequence = format!("e:{}", state.sequence);
        self.application_send(
            id,
            api::HostMessage::Event {
                sequence,
                event,
                data: job.into(),
            },
        )
    }
}

fn safe_label(value: &str, limit: usize) -> bool {
    !value.is_empty() && value.len() <= limit && !value.chars().any(char::is_control)
}

impl WorkspaceHost {
    fn application_view_request(
        &mut self,
        id: usize,
        request: api::Request,
    ) -> Result<api::ResultValue, api::Error> {
        use api::{ErrorCode as Code, Request};
        let fail = api::Error::new;
        if !self.app.plugins.instances[&id]
            .application
            .capabilities
            .contains("views")
        {
            return Err(fail(
                Code::CapabilityDenied,
                "Views capability was not granted",
            ));
        }
        if let Request::ViewCreate { model } = request {
            model.validate()?;
            self.reserve_application_payload(id, model.payload_bytes() * 2)?;
            let state = &mut self.app.plugins.instances.get_mut(&id).unwrap().application;
            if state.views.len() >= 16 {
                return Err(fail(Code::LimitExceeded, "View limit reached"));
            }
            state.retained_payload += model.payload_bytes() * 2;
            state.next_handle += 1;
            let handle = format!("v:{}:{}", state.generation, state.next_handle);
            let buffer = self.app.create_plugin_view(id, &handle, &model);
            self.app
                .plugins
                .instances
                .get_mut(&id)
                .unwrap()
                .application
                .views
                .insert(
                    handle.clone(),
                    crate::plugin::view::View {
                        buffer,
                        model: model.clone(),
                        revision: 1,
                        published: None,
                    },
                );
            return Ok(api::ResultValue::View {
                view: handle,
                revision: "m:1".into(),
                model,
            });
        }
        let (Request::ViewGet { view }
        | Request::ViewPublish { view, .. }
        | Request::ViewClose { view }
        | Request::PaneShow { view, .. }) = &request
        else {
            unreachable!()
        };
        let state = &self.app.plugins.instances[&id].application;
        let live = state
            .views
            .get(view)
            .ok_or_else(|| fail(Code::NotFound, "Unknown view"))?;
        let buffer = live.buffer;
        if self.app.host_buffer_is_closed(buffer) {
            return Err(fail(Code::Closed, "View is closed"));
        }
        match request {
            Request::ViewGet { view } => Ok(api::ResultValue::View {
                view,
                revision: format!("m:{}", live.revision),
                model: live.model.clone(),
            }),
            Request::PaneShow { invocation, .. } => {
                let context = state
                    .requests
                    .get(&invocation)
                    .ok_or_else(|| fail(Code::ContextChanged, "Invoking command finished"))?;
                self.app.plugin_foreground(context)?;
                self.app.present_plugin_view(buffer);
                Ok(api::ResultValue::Empty(api::Empty {}))
            }
            Request::ViewClose { view } => {
                self.app
                    .host_close_buffer(buffer, false)
                    .map_err(|_| fail(Code::Busy, "View cannot close"))?;
                let _ = view;
                Ok(api::ResultValue::Empty(api::Empty {}))
            }
            Request::ViewPublish {
                view,
                expected_revision,
                model,
            } => {
                if expected_revision != format!("m:{}", live.revision) {
                    return Err(fail(Code::Stale, "View model changed"));
                }
                model.validate()?;
                if live
                    .published
                    .is_some_and(|at| at.elapsed() < std::time::Duration::from_millis(100))
                {
                    return Err(fail(
                        Code::Busy,
                        "View publication is paced at ten updates per second",
                    ));
                }
                let old = live.model.clone();
                self.reserve_application_payload(
                    id,
                    (model.payload_bytes() * 2).saturating_sub(old.payload_bytes() * 2),
                )?;
                self.app
                    .plugins
                    .instances
                    .get_mut(&id)
                    .unwrap()
                    .application
                    .retained_payload =
                    self.app.plugins.instances[&id].application.retained_payload
                        - old.payload_bytes() * 2
                        + model.payload_bytes() * 2;
                self.app.publish_plugin_view(buffer, Some(&old), &model);
                let live = self
                    .app
                    .plugins
                    .instances
                    .get_mut(&id)
                    .unwrap()
                    .application
                    .views
                    .get_mut(&view)
                    .unwrap();
                live.revision += 1;
                live.published = Some(std::time::Instant::now());
                live.model = model.clone();
                Ok(api::ResultValue::View {
                    view,
                    revision: format!("m:{}", live.revision),
                    model,
                })
            }
            _ => unreachable!(),
        }
    }
}

impl WorkspaceHost {
    pub(super) fn reserve_application_payload(
        &self,
        owner: usize,
        bytes: usize,
    ) -> Result<(), api::Error> {
        // Leave room for bounded IO queues, one decoded model and publication copies.
        if self.app.plugins.instances[&owner]
            .application
            .retained_payload
            .saturating_add(bytes)
            > 48 * 1024 * 1024
            || self
                .app
                .plugins
                .instances
                .values()
                .map(|instance| instance.application.retained_payload)
                .sum::<usize>()
                .saturating_add(self.app.plugins.orphaned_payload)
                .saturating_add(bytes)
                > 160 * 1024 * 1024
        {
            return Err(api::Error::new(
                api::ErrorCode::LimitExceeded,
                "Retained application payload quota exceeded",
            ));
        }
        Ok(())
    }
}
