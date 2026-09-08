// SPDX-License-Identifier: MPL-2.0

use super::WorkspaceHost;
use crate::plugin::{self, application as api};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
const SAVE_CHARGE: usize = 16 * 1024 * 1024;
pub(super) struct PendingSave {
    pub owner: usize,
    pub generation: String,
    pub buffer: usize,
    pub cancelled: Arc<AtomicBool>,
    pub orphaned: bool,
}
impl WorkspaceHost {
    pub(super) fn application_document_request(
        &mut self,
        owner: usize,
        id: String,
        request: api::Request,
    ) -> anyhow::Result<()> {
        match self.start_document_operation(owner, request) {
            Ok((result, job)) => {
                self.application_local_reply(owner, id, Ok(result))?;
                if let Some(job) = job {
                    self.application_job_event(owner, "job.changed", job)?;
                }
                Ok(())
            }
            Err(error) => self.application_local_reply(owner, id, Err(error)),
        }
    }
    fn start_document_operation(
        &mut self,
        owner: usize,
        request: api::Request,
    ) -> Result<(api::ResultValue, Option<api::Job>), api::Error> {
        let fail = api::Error::new;
        let (handle, revision, save) = match request {
            api::Request::BufferSave {
                buffer,
                expected_revision,
            } => (buffer, expected_revision, true),
            api::Request::BufferClose {
                buffer,
                expected_revision,
            } => (buffer, expected_revision, false),
            _ => unreachable!(),
        };
        let state = &self.app.plugins.instances[&owner].application;
        if !state.capabilities.contains("documents") {
            return Err(fail(
                api::ErrorCode::CapabilityDenied,
                "Documents capability was not granted",
            ));
        }
        let buffer = *state
            .buffers
            .get(&handle)
            .ok_or_else(|| fail(api::ErrorCode::NotFound, "Unknown document"))?;
        if self.app.host_buffer_is_closed(buffer) {
            return Err(fail(api::ErrorCode::Closed, "Document closed"));
        }
        if format!("r:{}", self.app.buffers[buffer].revision()) != revision {
            return Err(fail(api::ErrorCode::Stale, "Document changed"));
        }
        if self.app.document_mutation_pending(buffer) {
            return Err(fail(api::ErrorCode::Busy, "Document save is pending"));
        }
        if !save {
            if self.app.buffers[buffer].dirty {
                return Err(fail(
                    api::ErrorCode::Conflict,
                    "Save or explicitly discard the document before closing",
                ));
            }
            self.app
                .host_close_buffer(buffer, false)
                .map_err(|_| fail(api::ErrorCode::Conflict, "Document could not close"))?;
            return Ok((api::ResultValue::Empty(api::Empty {}), None));
        }
        if self.app.buffers[buffer].provider().is_some() {
            if !state.capabilities.contains("jobs") {
                return Err(fail(
                    api::ErrorCode::CapabilityDenied,
                    "Resource save also requires jobs capability",
                ));
            }
            let job = self.start_provider_write(owner, buffer, None)?;
            return Ok((api::ResultValue::Job(job.clone()), Some(job)));
        }
        let status = self.app.buffers[buffer].external_file_status();
        if status.is_stale() && status != crate::buffer::ExternalFileStatus::Deleted {
            return Err(fail(
                api::ErrorCode::Conflict,
                "Document changed on disk; inspect and reconcile before saving",
            ));
        }
        if self.app.buffers[buffer].is_read_only() {
            return Err(fail(api::ErrorCode::ReadOnly, "Document is read-only"));
        }
        if self.app.buffers[buffer].text().len_bytes()
            > crate::plugin::filesystem::MAX_DOCUMENT_BYTES
        {
            return Err(fail(
                api::ErrorCode::LimitExceeded,
                "Document exceeds local save limit",
            ));
        }
        self.app.buffers[buffer]
            .prepare_document_save()
            .map_err(|_| {
                fail(
                    api::ErrorCode::Unsupported,
                    "Save requires an ordinary file document",
                )
            })?;
        self.reserve_application_payload(owner, SAVE_CHARGE)?;
        let sender = self
            .plugin_events_sender
            .clone()
            .ok_or_else(|| fail(api::ErrorCode::Unavailable, "Document IO is unavailable"))?;
        let runtime = tokio::runtime::Handle::try_current().map_err(|_| {
            fail(
                api::ErrorCode::Unavailable,
                "Document runtime is unavailable",
            )
        })?;
        let permit = self
            .plugin_local_slots
            .get_or_insert_with(|| Arc::new(tokio::sync::Semaphore::new(16)))
            .clone()
            .try_acquire_owned()
            .map_err(|_| fail(api::ErrorCode::Busy, "Document IO is busy"))?;
        let (api::ResultValue::Job(job), _) = self.application_request(
            owner,
            api::Request::JobCreate {
                title: "Save document".into(),
                deadline_seconds: 60,
            },
        )?
        else {
            unreachable!()
        };
        let work = self.app.prepare_plugin_document_save(buffer);
        let state = &mut self
            .app
            .plugins
            .instances
            .get_mut(&owner)
            .unwrap()
            .application;
        state.retained_payload += SAVE_CHARGE;
        let cancelled = Arc::new(AtomicBool::new(false));
        self.document_saves.insert(
            job.job.clone(),
            PendingSave {
                owner,
                generation: state.generation.clone(),
                buffer,
                cancelled: cancelled.clone(),
                orphaned: false,
            },
        );
        self.app.plugins.document_saves.insert(buffer);
        let root = self.app.project_root.clone();
        let token = job.job.clone();
        let worker_failure = cancelled.clone();
        let worker = runtime.spawn_blocking(move || {
            if cancelled.load(Ordering::SeqCst) {
                Ok(None)
            } else {
                work.run(&root).map(Some).map_err(|e| e.to_string())
            }
        });
        runtime.spawn(async move {
            let result = worker.await.unwrap_or_else(|error| {
                // A panicking worker may have installed its snapshot before failing.
                worker_failure.store(true, Ordering::SeqCst);
                Err(format!("Document worker failed: {error}"))
            });
            let _ = sender
                .send(plugin::Event {
                    plugin: owner,
                    result: Ok(plugin::ClientMessage::DocumentSaved {
                        job: token,
                        result,
                        _permit: permit,
                    }),
                })
                .await;
        });
        Ok((api::ResultValue::Job(job.clone()), Some(job)))
    }
    pub(super) fn orphan_document_saves(&mut self, owner: usize) {
        for pending in self
            .document_saves
            .values_mut()
            .filter(|p| p.owner == owner && !p.orphaned)
        {
            pending.orphaned = true;
            pending.cancelled.store(true, Ordering::SeqCst);
            self.app.plugins.orphaned_payload += SAVE_CHARGE;
        }
    }
    pub(super) fn complete_document_save(
        &mut self,
        token: String,
        result: Result<Option<crate::buffer::SavedDocument>, String>,
    ) {
        let Some(pending) = self.document_saves.remove(&token) else {
            return;
        };
        self.app.plugins.document_saves.remove(&pending.buffer);
        let live = self
            .app
            .plugins
            .instances
            .get(&pending.owner)
            .is_some_and(|i| i.application.generation == pending.generation);
        if pending.orphaned {
            self.app.plugins.orphaned_payload -= SAVE_CHARGE;
        } else if live {
            self.app
                .plugins
                .instances
                .get_mut(&pending.owner)
                .unwrap()
                .application
                .retained_payload -= SAVE_CHARGE;
        }
        let cancelled = pending.cancelled.load(Ordering::SeqCst);
        let terminal = self
            .app
            .finish_plugin_document_save(pending.buffer, cancelled, result);
        if live {
            let state = &mut self
                .app
                .plugins
                .instances
                .get_mut(&pending.owner)
                .unwrap()
                .application;
            if let Some(job) = state.jobs.get_mut(&token) {
                job.state = terminal;
                let job = job.clone();
                state.finished_jobs.push_back(token.clone());
                if self
                    .plugin_send(
                        pending.owner,
                        plugin::HostMessage::Deadline {
                            token,
                            after_ms: None,
                        },
                    )
                    .is_err()
                    || self
                        .application_job_event(pending.owner, "job.changed", job)
                        .is_err()
                {
                    self.stop_plugin(pending.owner, "document result consumer is too slow");
                }
            }
        }
        self.reconcile_wait_requests();
        self.app.plugins.presentation_dirty = true;
    }
}
