// SPDX-License-Identifier: MPL-2.0

use super::WorkspaceHost;
use crate::plugin::{application as api, filesystem as local, staging};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

impl WorkspaceHost {
    pub(super) fn plugin_staging_task(
        &self,
        owner: usize,
        request: api::Request,
        pending: &mut local::Pending,
    ) -> Result<local::Task, api::Error> {
        use api::{Error, ErrorCode as Code, Request};
        let state = &self.app.plugins.instances[&owner].application;
        if !state.capabilities.contains("jobs") {
            return Err(Error::new(
                Code::CapabilityDenied,
                "Download staging also requires jobs capability",
            ));
        }
        let running = |job: &str| {
            if !state
                .jobs
                .get(job)
                .is_some_and(|job| job.state == api::JobState::Running)
            {
                return Err(Error::new(Code::Cancelled, "Download job is not running"));
            }
            if self.host_job_cancellation(owner, job).is_some()
                || self.provider_reads.contains_key(job)
                || self.provider_writes.contains_key(job)
            {
                return Err(Error::new(
                    Code::Conflict,
                    "Download requires a plugin-owned job",
                ));
            }
            Ok(())
        };
        match request {
            Request::StagingCreate { job, bytes } => {
                running(&job)?;
                if bytes > staging::MAX_BYTES {
                    return Err(Error::new(Code::LimitExceeded, "Download exceeds 8 MiB"));
                }
                let creating = state
                    .local_requests
                    .values()
                    .filter(|pending| {
                        pending.staging_job.is_some() && pending.staging_handle.is_none()
                    })
                    .count();
                if state.staging.len() + creating >= staging::MAX_STAGING {
                    return Err(Error::new(
                        Code::LimitExceeded,
                        "Two download staging files are already retained",
                    ));
                }
                let cancelled = Arc::new(AtomicBool::new(false));
                pending.staging_job = Some(job);
                pending.cancelled = Some(cancelled.clone());
                pending.charge = staging::STAGING_CHARGE;
                Ok(local::Task::DownloadCreate {
                    root: self.app.state_root.clone(),
                    bytes,
                    cancelled,
                })
            }
            Request::StagingPrepare {
                staging: handle,
                directory,
                expected_revision,
                destination,
                sha256,
            } => {
                let issued = state
                    .staging
                    .get(&handle)
                    .ok_or_else(|| Error::new(Code::NotFound, "Unknown download staging file"))?;
                running(&issued.job)?;
                if issued.busy {
                    return Err(Error::new(
                        Code::Busy,
                        "Download sealing is already pending",
                    ));
                }
                if state.plans.len() >= local::MAX_PLANS {
                    return Err(Error::new(
                        Code::LimitExceeded,
                        "Prepared plan limit reached",
                    ));
                }
                let directory = state
                    .directories
                    .get(&directory)
                    .ok_or_else(|| Error::new(Code::NotFound, "Unknown directory"))?;
                if directory.revision != expected_revision {
                    return Err(Error::new(Code::Stale, "Directory listing changed"));
                }
                if sha256.len() != 64
                    || !sha256
                        .bytes()
                        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
                {
                    return Err(Error::new(
                        Code::InvalidArgument,
                        "Expected a lowercase SHA-256 digest",
                    ));
                }
                if destination.len() > 4096 {
                    return Err(Error::new(
                        Code::InvalidArgument,
                        "Destination path exceeds its limit",
                    ));
                }
                pending.staging_job = Some(issued.job.clone());
                pending.staging_handle = Some(handle);
                pending.cancelled = Some(issued.cancelled.clone());
                pending.charge = staging::PREPARE_CHARGE;
                Ok(local::Task::DownloadPrepare {
                    root: self.app.project_root.clone(),
                    download: issued.download.clone(),
                    directory: directory.clone(),
                    destination,
                    sha256,
                    cancelled: issued.cancelled.clone(),
                })
            }
            _ => unreachable!(),
        }
    }

    pub(super) fn close_plugin_staging(&mut self, owner: usize, handle: &str) {
        let state = &mut self
            .app
            .plugins
            .instances
            .get_mut(&owner)
            .unwrap()
            .application;
        if let Some(issued) = state.staging.remove(handle) {
            issued.cancelled.store(true, Ordering::SeqCst);
            state.retained_payload -= staging::STAGING_CHARGE;
        }
    }

    pub(super) fn retire_plugin_staging_job(&mut self, owner: usize, job: &str) {
        let state = &mut self
            .app
            .plugins
            .instances
            .get_mut(&owner)
            .unwrap()
            .application;
        for pending in state
            .local_requests
            .values()
            .filter(|pending| pending.staging_job.as_deref() == Some(job))
        {
            if let Some(cancelled) = &pending.cancelled {
                cancelled.store(true, Ordering::SeqCst);
            }
        }
        state.staging.retain(|_, issued| {
            if issued.job == job {
                issued.cancelled.store(true, Ordering::SeqCst);
                state.retained_payload -= staging::STAGING_CHARGE;
                false
            } else {
                true
            }
        });
        state.staging_plans.retain(|plan, source_job| {
            if source_job == job {
                if state.plans.remove(plan).is_some() {
                    state.retained_payload -= local::DIRECTORY_CHARGE;
                }
                false
            } else {
                true
            }
        });
    }
}
