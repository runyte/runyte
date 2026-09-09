// SPDX-License-Identifier: MPL-2.0
use super::WorkspaceHost;
use crate::plugin::{self, application as api, filesystem as local};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU8, Ordering},
};
const EXTRA_CHARGE: usize = 16 * 1024 * 1024;
const APPLY_CHARGE: usize = EXTRA_CHARGE + local::DIRECTORY_CHARGE;
pub(super) struct PendingApply {
    pub owner: usize,
    pub generation: String,
    pub job: String,
    pub plan: String,
    pub cancelled: Arc<AtomicBool>,
    pub phase: Arc<AtomicU8>,
    pub orphaned: bool,
}
impl WorkspaceHost {
    pub(super) fn cancel_queued_filesystem_job(
        &self,
        owner: usize,
        token: &str,
    ) -> Result<(), api::Error> {
        if let Some(pending) = self
            .filesystem_apply
            .as_ref()
            .filter(|p| p.owner == owner && p.job == token)
        {
            match pending
                .phase
                .compare_exchange(0, 2, Ordering::SeqCst, Ordering::SeqCst)
            {
                Ok(_) | Err(2) => {}
                Err(_) => {
                    return Err(api::Error::new(
                        api::ErrorCode::Conflict,
                        "Filesystem mutation already started; wait for its result",
                    ));
                }
            }
        }
        Ok(())
    }
    pub(super) fn host_job_cancellation(
        &self,
        owner: usize,
        token: &str,
    ) -> Option<Arc<AtomicBool>> {
        let generation = &self
            .app
            .plugins
            .instances
            .get(&owner)?
            .application
            .generation;
        self.document_saves
            .get(token)
            .filter(|p| p.owner == owner && &p.generation == generation)
            .map(|p| p.cancelled.clone())
            .or_else(|| {
                self.filesystem_apply
                    .as_ref()
                    .filter(|p| p.job == token && p.owner == owner && &p.generation == generation)
                    .map(|p| p.cancelled.clone())
            })
    }
    pub(super) fn start_plugin_filesystem_apply(
        &mut self,
        deletion: crate::fs_plan::DeletionMode,
    ) -> anyhow::Result<()> {
        anyhow::ensure!(
            !self.app.plugins.filesystem_applying && self.app.plugins.document_saves.is_empty(),
            "Wait for pending filesystem writes"
        );
        let (owner, plan, revision, _) = self
            .app
            .plugins
            .filesystem_confirmation
            .clone()
            .ok_or_else(|| anyhow::anyhow!("Confirmation closed"))?;
        anyhow::ensure!(
            self.app.plugin_filesystem_confirmation_matches(revision),
            "Confirmation changed"
        );
        self.reserve_application_payload(owner, EXTRA_CHARGE)
            .map_err(|e| anyhow::anyhow!(e.message))?;
        let inputs = self.app.plugin_filesystem_inputs()?;
        let sender = self
            .plugin_events_sender
            .clone()
            .ok_or_else(|| anyhow::anyhow!("Filesystem IO is unavailable"))?;
        let runtime = tokio::runtime::Handle::try_current()?;
        let permit = self
            .plugin_local_slots
            .get_or_insert_with(|| Arc::new(tokio::sync::Semaphore::new(16)))
            .clone()
            .try_acquire_owned()?;
        let created = self.application_request(
            owner,
            api::Request::JobCreate {
                title: "Apply filesystem changes".into(),
                deadline_seconds: 60,
            },
        );
        let (api::ResultValue::Job(job), _) = (match created {
            Ok(result) => result,
            Err(error) => {
                if error.code == api::ErrorCode::Unavailable {
                    self.stop_plugin(
                        owner,
                        "filesystem job setup could not reach the application",
                    );
                }
                anyhow::bail!(error.message);
            }
        }) else {
            unreachable!()
        };
        let confirmation = self.app.fs_confirmation.take().unwrap();
        self.app.plugins.filesystem_confirmation = None;
        self.app.plugins.filesystem_applying = true;
        self.app.plugins.presentation_dirty = true;
        let state = &mut self
            .app
            .plugins
            .instances
            .get_mut(&owner)
            .unwrap()
            .application;
        state.retained_payload += EXTRA_CHARGE;
        let cancelled = Arc::new(AtomicBool::new(false));
        let phase = Arc::new(AtomicU8::new(0));
        self.filesystem_apply = Some(PendingApply {
            owner,
            generation: state.generation.clone(),
            job: job.job.clone(),
            plan: plan.clone(),
            cancelled: cancelled.clone(),
            phase: phase.clone(),
            orphaned: false,
        });
        let trash = self.app.plugin_trash_backend();
        let view = self.app.plugin_listing_view();
        let token = job.job.clone();
        let root = self.app.project_root.clone();
        let worker = runtime.spawn_blocking(move || {
            crate::app::App::run_plugin_filesystem(
                confirmation.plan,
                deletion,
                trash,
                inputs,
                view,
                phase,
                root,
            )
        });
        runtime.spawn(async move {
            let result = worker
                .await
                .map_err(|error| format!("Filesystem worker failed: {error}"));
            let _ = sender
                .send(plugin::Event {
                    plugin: owner,
                    result: Ok(plugin::ClientMessage::FilesystemApplied {
                        job: token,
                        result,
                        _permit: permit,
                    }),
                })
                .await;
        });
        if self
            .application_job_event(owner, "job.changed", job.clone())
            .is_err()
            || self
                .filesystem_event(
                    owner,
                    "filesystem.started",
                    api::EventData::FilesystemStarted { plan, job: job.job },
                )
                .is_err()
        {
            self.stop_plugin(owner, "filesystem result consumer is too slow");
        }
        Ok(())
    }
    pub(super) fn orphan_plugin_filesystem_apply(&mut self, owner: usize) {
        if let Some(pending) = self
            .filesystem_apply
            .as_mut()
            .filter(|p| p.owner == owner && !p.orphaned)
        {
            let _ = pending
                .phase
                .compare_exchange(0, 2, Ordering::SeqCst, Ordering::SeqCst);
            pending.orphaned = true;
            pending.cancelled.store(true, Ordering::SeqCst);
            self.app.plugins.orphaned_payload += APPLY_CHARGE;
        }
    }
    fn filesystem_event(
        &mut self,
        owner: usize,
        event: &'static str,
        data: api::EventData,
    ) -> anyhow::Result<()> {
        let state = &mut self
            .app
            .plugins
            .instances
            .get_mut(&owner)
            .unwrap()
            .application;
        state.sequence += 1;
        let sequence = format!("e:{}", state.sequence);
        self.application_send(
            owner,
            api::HostMessage::Event {
                sequence,
                event,
                data,
            },
        )
    }
    pub(super) fn complete_plugin_filesystem_apply(
        &mut self,
        token: String,
        result: Result<local::Applied, String>,
    ) {
        if !self
            .filesystem_apply
            .as_ref()
            .is_some_and(|p| p.job == token)
        {
            return;
        }
        let pending = self.filesystem_apply.take().unwrap();
        self.app.plugins.filesystem_applying = false;
        let live = self
            .app
            .plugins
            .instances
            .get(&pending.owner)
            .is_some_and(|i| i.application.generation == pending.generation);
        if pending.orphaned {
            self.app.plugins.orphaned_payload -= APPLY_CHARGE;
        } else if live {
            self.app
                .plugins
                .instances
                .get_mut(&pending.owner)
                .unwrap()
                .application
                .retained_payload -= APPLY_CHARGE;
        }
        let (finished, terminal) = self
            .app
            .finish_plugin_filesystem_apply(pending.plan, result);
        if live {
            let state = &mut self
                .app
                .plugins
                .instances
                .get_mut(&pending.owner)
                .unwrap()
                .application;
            let job = state.jobs.get_mut(&token).unwrap();
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
                || self
                    .filesystem_event(
                        pending.owner,
                        "filesystem.finished",
                        api::EventData::FilesystemFinished(finished),
                    )
                    .is_err()
            {
                self.stop_plugin(pending.owner, "filesystem result consumer is too slow");
            }
        }
        self.reconcile_wait_requests();
        self.app.plugins.presentation_dirty = true;
    }
}
