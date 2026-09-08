// SPDX-License-Identifier: MPL-2.0

use super::WorkspaceHost;
use crate::plugin::{self, application as api, filesystem as local};
use api::{Error, ErrorCode as Code, Request, ResultValue};

impl WorkspaceHost {
    pub(super) fn application_local_reply(
        &mut self,
        owner: usize,
        request: String,
        result: Result<ResultValue, Error>,
    ) -> anyhow::Result<()> {
        self.application_send(
            owner,
            api::HostMessage::Response {
                id: request,
                outcome: match result {
                    Ok(result) => api::Response::Success { result },
                    Err(error) => api::Response::Failure { error },
                },
            },
        )
    }

    pub(super) fn application_filesystem_request(
        &mut self,
        owner: usize,
        request: &str,
        operation: Request,
    ) -> Result<Option<ResultValue>, Error> {
        let fail = Error::new;
        let state = &self.app.plugins.instances[&owner].application;
        let capability = if matches!(
            operation,
            Request::BufferOpen { .. } | Request::BufferCreate { .. }
        ) {
            "documents"
        } else {
            "filesystem"
        };
        if !state.capabilities.contains(capability) {
            return Err(fail(Code::CapabilityDenied, "Capability was not granted"));
        }
        let mut pending = local::Pending {
            staging_job: None,
            staging_handle: None,
            cancelled: None,
            creating: false,
            invocation: None,
            offset: 0,
            limit: 128,
            charge: local::DIRECTORY_CHARGE,
            expected_revision: None,
        };
        let root = self.app.project_root.clone();
        let task =
            match operation {
                operation @ (Request::StagingCreate { .. } | Request::StagingPrepare { .. }) => {
                    self.plugin_staging_task(owner, operation, &mut pending)?
                }
                Request::StagingClose { staging } => {
                    self.close_plugin_staging(owner, &staging);
                    return Ok(Some(ResultValue::Empty(api::Empty {})));
                }
                Request::FilesystemStat {
                    path,
                    expected_revision,
                } => {
                    if path.len() > 4096
                        || expected_revision
                            .as_ref()
                            .is_some_and(|revision| revision.len() > 128)
                    {
                        return Err(fail(Code::InvalidArgument, "Invalid stat path or revision"));
                    }
                    pending.charge = 64 * 1024;
                    pending.expected_revision = expected_revision;
                    local::Task::Stat { root, path }
                }
                Request::FilesystemList {
                    path,
                    offset,
                    limit,
                    expected_revision,
                } => {
                    if !(1..=128).contains(&limit) {
                        return Err(fail(Code::InvalidArgument, "Page limit must be 1–128"));
                    }
                    pending.offset = offset;
                    pending.limit = limit;
                    pending.expected_revision = expected_revision;
                    local::Task::List { root, path }
                }
                Request::FilesystemPrepare {
                    directory,
                    expected_revision,
                    intent,
                } => {
                    let directory = state
                        .directories
                        .get(&directory)
                        .ok_or_else(|| fail(Code::NotFound, "Unknown directory"))?;
                    if directory.revision != expected_revision {
                        return Err(fail(Code::Stale, "Directory listing changed"));
                    }
                    if state.plans.len() >= local::MAX_PLANS {
                        return Err(fail(Code::LimitExceeded, "Prepared plan limit reached"));
                    }
                    local::Task::Prepare {
                        root,
                        directory: directory.clone(),
                        intent,
                    }
                }
                Request::BufferCreate {
                    path,
                    text,
                    invocation,
                } => {
                    if let Some(invocation) = &invocation {
                        let context = state.requests.get(invocation).ok_or_else(|| {
                            fail(Code::ContextChanged, "Invoking command finished")
                        })?;
                        self.app.plugin_foreground(context)?;
                    }
                    if text.len() > 512 * 1024 {
                        return Err(fail(
                            Code::LimitExceeded,
                            "Initial text exceeds document limit",
                        ));
                    }
                    pending.creating = true;
                    pending.invocation = invocation;
                    pending.charge = local::MAX_DOCUMENT_BYTES * 2;
                    local::Task::Create { root, path, text }
                }
                Request::BufferOpen { path, invocation } => {
                    if let Some(invocation) = &invocation {
                        let context = state.requests.get(invocation).ok_or_else(|| {
                            fail(Code::ContextChanged, "Invoking command finished")
                        })?;
                        self.app.plugin_foreground(context)?;
                    }
                    pending.invocation = invocation;
                    pending.charge = local::MAX_DOCUMENT_BYTES * 2;
                    local::Task::Open { root, path }
                }
                Request::FilesystemApply { plan, invocation } => {
                    if !state.capabilities.contains("jobs") {
                        return Err(fail(
                            Code::CapabilityDenied,
                            "Filesystem application also requires jobs capability",
                        ));
                    }
                    if self.app.plugins.filesystem_applying {
                        return Err(fail(Code::Busy, "Filesystem application is pending"));
                    }
                    let context = state
                        .requests
                        .get(&invocation)
                        .ok_or_else(|| fail(Code::ContextChanged, "Invoking command finished"))?;
                    self.app.plugin_foreground(context)?;
                    if self.app.plugin_has_input_surface() {
                        return Err(fail(
                            Code::Busy,
                            "An input surface already owns the frontend",
                        ));
                    }
                    let prepared = state
                        .plans
                        .get(&plan)
                        .ok_or_else(|| fail(Code::NotFound, "Unknown prepared plan"))?
                        .clone();
                    self.app
                        .present_plugin_filesystem(owner, plan.clone(), prepared);
                    let state = &mut self
                        .app
                        .plugins
                        .instances
                        .get_mut(&owner)
                        .unwrap()
                        .application;
                    state.plans.remove(&plan);
                    state.staging_plans.remove(&plan);
                    // Charge stays with the displayed confirmation until it settles.
                    return Ok(Some(ResultValue::Empty(api::Empty {})));
                }
                Request::FilesystemCancel { plan } => {
                    self.app.cancel_plugin_filesystem(owner, Some(&plan));
                    let state = &mut self
                        .app
                        .plugins
                        .instances
                        .get_mut(&owner)
                        .unwrap()
                        .application;
                    if state.plans.remove(&plan).is_some() {
                        state.retained_payload -= local::DIRECTORY_CHARGE;
                    }
                    state.staging_plans.remove(&plan);
                    return Ok(Some(ResultValue::Empty(api::Empty {})));
                }
                Request::FilesystemRelease { directory } => {
                    let state = &mut self
                        .app
                        .plugins
                        .instances
                        .get_mut(&owner)
                        .unwrap()
                        .application;
                    if state.directories.remove(&directory).is_some() {
                        state.retained_payload -= local::DIRECTORY_CHARGE;
                    }
                    return Ok(Some(ResultValue::Empty(api::Empty {})));
                }
                _ => unreachable!(),
            };
        if state.local_requests.len() >= 2 {
            return Err(fail(
                Code::Busy,
                "Two local filesystem requests are already pending",
            ));
        }
        self.reserve_application_payload(owner, pending.charge)?;
        let sender = self
            .plugin_events_sender
            .clone()
            .ok_or_else(|| fail(Code::Unavailable, "Local operation service is unavailable"))?;
        let runtime = tokio::runtime::Handle::try_current()
            .map_err(|_| fail(Code::Unavailable, "Local operation runtime is unavailable"))?;
        let slots = self
            .plugin_local_slots
            .get_or_insert_with(|| std::sync::Arc::new(tokio::sync::Semaphore::new(16)))
            .clone();
        let permit = slots
            .try_acquire_owned()
            .map_err(|_| fail(Code::Busy, "Local operation service is busy"))?;
        let state = &mut self
            .app
            .plugins
            .instances
            .get_mut(&owner)
            .unwrap()
            .application;
        let generation = state.generation.clone();
        if let Some(handle) = &pending.staging_handle {
            state.staging.get_mut(handle).unwrap().busy = true;
        }
        state.retained_payload += pending.charge;
        state.local_requests.insert(request.into(), pending);
        let request = request.to_owned();
        // The permit remains with the blocking read and then its queued result,
        // even when the owner stops while the OS operation is still running.
        let work = runtime.spawn_blocking(move || task.run());
        runtime.spawn(async move {
            let result = work.await.unwrap_or_else(|_| {
                Err(Error::new(Code::Internal, "Local filesystem worker failed"))
            });
            let _ = sender
                .send(plugin::Event {
                    plugin: owner,
                    result: Ok(plugin::ClientMessage::Local {
                        generation,
                        request,
                        result,
                        _permit: permit,
                    }),
                })
                .await;
        });
        Ok(None)
    }

    pub(super) fn application_local_result(
        &mut self,
        owner: usize,
        generation: String,
        request: String,
        result: Result<local::Prepared, Error>,
    ) -> anyhow::Result<()> {
        let state = &mut self
            .app
            .plugins
            .instances
            .get_mut(&owner)
            .unwrap()
            .application;
        if state.generation != generation {
            return Ok(());
        }
        let Some(pending) = state.local_requests.remove(&request) else {
            return Ok(());
        };
        state.retained_payload -= pending.charge;
        if let Some(handle) = &pending.staging_handle
            && let Some(issued) = state.staging.get_mut(handle)
        {
            issued.busy = false;
        }
        if let Some(job) = &pending.staging_job
            && (!state
                .jobs
                .get(job)
                .is_some_and(|job| job.state == api::JobState::Running)
                || pending
                    .cancelled
                    .as_ref()
                    .is_some_and(|cancelled| cancelled.load(std::sync::atomic::Ordering::SeqCst))
                || pending
                    .staging_handle
                    .as_ref()
                    .is_some_and(|handle| !state.staging.contains_key(handle)))
        {
            return self.application_local_reply(
                owner,
                request,
                Err(Error::new(
                    Code::Cancelled,
                    "Download was cancelled or released",
                )),
            );
        }
        let result =
            result.and_then(|prepared| self.publish_application_local(owner, pending, prepared));
        self.application_local_reply(owner, request, result)
    }

    fn publish_application_local(
        &mut self,
        owner: usize,
        pending: local::Pending,
        prepared: local::Prepared,
    ) -> Result<ResultValue, Error> {
        let fail = Error::new;
        match prepared {
            local::Prepared::Download(download) => {
                let state = &mut self
                    .app
                    .plugins
                    .instances
                    .get_mut(&owner)
                    .unwrap()
                    .application;
                state.next_handle += 1;
                let handle = format!("t:{}:{}", state.generation, state.next_handle);
                let path = download
                    .path()
                    .to_str()
                    .ok_or_else(|| fail(Code::Unsupported, "Download staging path is not UTF-8"))?
                    .to_owned();
                if path.len() > 4096 || path.chars().any(char::is_control) {
                    return Err(fail(
                        Code::Unsupported,
                        "Download staging path cannot be represented by the application protocol",
                    ));
                }
                state.staging.insert(
                    handle.clone(),
                    crate::plugin::staging::Issued {
                        job: pending.staging_job.unwrap(),
                        download,
                        busy: false,
                        cancelled: pending.cancelled.unwrap(),
                    },
                );
                state.retained_payload += crate::plugin::staging::STAGING_CHARGE;
                Ok(ResultValue::Staging {
                    staging: handle,
                    path,
                })
            }
            local::Prepared::Stat(stat) => {
                if pending
                    .expected_revision
                    .as_ref()
                    .is_some_and(|expected| *expected != stat.revision)
                {
                    return Err(fail(Code::Stale, "Filesystem metadata changed"));
                }
                Ok(ResultValue::Stat(stat))
            }
            local::Prepared::Directory(directory) => {
                if pending
                    .expected_revision
                    .as_ref()
                    .is_some_and(|revision| revision != &directory.revision)
                {
                    return Err(fail(Code::Stale, "Directory changed between pages"));
                }
                let entries = directory.page(pending.offset, pending.limit);
                let next = (pending.offset.saturating_add(entries.len())
                    < directory.snapshot.entries().len())
                .then_some(pending.offset.saturating_add(entries.len()));
                let revision = directory.revision.clone();
                let state = &mut self
                    .app
                    .plugins
                    .instances
                    .get_mut(&owner)
                    .unwrap()
                    .application;
                let existing = state
                    .directories
                    .iter()
                    .find(|(_, old)| old.path == directory.path)
                    .map(|(key, _)| key.clone());
                if existing.is_none() && state.directories.len() >= local::MAX_DIRECTORIES {
                    return Err(fail(
                        Code::LimitExceeded,
                        "Directory snapshot limit reached; release an old directory",
                    ));
                }
                let handle = existing.unwrap_or_else(|| {
                    state.next_handle += 1;
                    state.retained_payload += local::DIRECTORY_CHARGE;
                    format!("d:{}:{}", state.generation, state.next_handle)
                });
                state.directories.insert(handle.clone(), directory);
                Ok(ResultValue::Directory {
                    directory: handle,
                    revision,
                    entries,
                    next,
                })
            }
            local::Prepared::Plan(plan) => {
                let state = &mut self
                    .app
                    .plugins
                    .instances
                    .get_mut(&owner)
                    .unwrap()
                    .application;
                if state.plans.len() >= local::MAX_PLANS {
                    return Err(fail(Code::LimitExceeded, "Prepared plan limit reached"));
                }
                let operations = plan.lines();
                state.next_handle += 1;
                let handle = format!("f:{}:{}", state.generation, state.next_handle);
                if let Some(job) = pending.staging_job {
                    let issued = state
                        .staging
                        .remove(pending.staging_handle.as_ref().unwrap())
                        .unwrap();
                    issued
                        .cancelled
                        .store(true, std::sync::atomic::Ordering::SeqCst);
                    state.retained_payload -= crate::plugin::staging::STAGING_CHARGE;
                    state.staging_plans.insert(handle.clone(), job);
                }
                state.plans.insert(handle.clone(), plan);
                state.retained_payload += local::DIRECTORY_CHARGE;
                Ok(ResultValue::FilesystemPlan {
                    plan: handle,
                    operations,
                })
            }
            local::Prepared::Document(buffer) => {
                if self.app.plugins.filesystem_applying {
                    return Err(fail(
                        Code::Busy,
                        "Wait for filesystem changes before publishing a document",
                    ));
                }
                if let Some(invocation) = &pending.invocation {
                    let context = self.app.plugins.instances[&owner]
                        .application
                        .requests
                        .get(invocation)
                        .ok_or_else(|| fail(Code::ContextChanged, "Invoking command finished"))?;
                    self.app.plugin_foreground(context)?;
                }
                let path = buffer.path.as_deref().expect("prepared ordinary file");
                if pending.creating && self.app.plugin_document_path_is_open(path) {
                    return Err(fail(Code::Conflict, "Document path is already open"));
                }
                crate::path_safety::ensure_within_root(&self.app.project_root, path)
                    .map_err(|_| fail(Code::Conflict, "Document path changed"))?;
                let state = &self.app.plugins.instances[&owner].application;
                if state.buffers.len() >= 1024 {
                    return Err(fail(Code::LimitExceeded, "Buffer handle limit reached"));
                }
                let index = self
                    .app
                    .install_plugin_document(*buffer, pending.invocation.is_some());
                let handle = self
                    .app
                    .plugins
                    .instances
                    .get_mut(&owner)
                    .unwrap()
                    .application
                    .buffer_handle(index)?;
                Ok(ResultValue::Opened {
                    buffer: handle,
                    revision: format!("r:{}", self.app.buffers[index].revision()),
                })
            }
        }
    }

    pub(super) fn sync_plugin_filesystem(&mut self) {
        self.app.sync_plugin_filesystem_confirmation();
        if let Some(deletion) = self.app.plugins.filesystem_accepted.take()
            && let Err(error) = self.start_plugin_filesystem_apply(deletion)
        {
            self.app.plugin_filesystem_start_failed(error.to_string());
        }
        for (owner, finished) in std::mem::take(&mut self.app.plugins.filesystem_finished) {
            let Some(instance) = self.app.plugins.instances.get_mut(&owner) else {
                continue;
            };
            instance.application.retained_payload -= local::DIRECTORY_CHARGE;
            instance.application.sequence += 1;
            let sequence = format!("e:{}", instance.application.sequence);
            if self
                .application_send(
                    owner,
                    api::HostMessage::Event {
                        sequence,
                        event: "filesystem.finished",
                        data: api::EventData::FilesystemFinished(finished),
                    },
                )
                .is_err()
            {
                self.stop_plugin(owner, "filesystem result consumer is too slow");
            }
        }
    }
}
