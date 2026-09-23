// SPDX-License-Identifier: MPL-2.0

//! Exact native catalog actions. The caller owns this snapshot through every
//! operation; a later service will own it off the editor event loop.

use super::{
    windows_catalog::{self, HistorySnapshot, HistoryTarget},
    windows_endpoint::{CandidateOrigin, Removal, StoppedNameEdit},
    windows_lifecycle::{
        await_host_stopped, force_shutdown_host, remove_stopped_observation, rename_host,
        shutdown_host, terminate_incompatible_host,
    },
    windows_location::{DiscoveryScope, KnownReadLocation},
};
use anyhow::{Context, Result, ensure};
use std::path::Path;

const MAX_FAILURE_TEXT_BYTES: usize = 512;
const MAX_STOP_ALL_FAILURE_BYTES: usize = 64 * 1024;

/// A selector is resolved once against one complete catalog. Its working
/// directory gives relative paths meaning, but never invents a current project.
#[derive(Clone, Copy)]
pub struct UserSelector<'a> {
    pub selector: &'a Path,
    pub working_directory: Option<&'a Path>,
}

#[derive(Debug, Default)]
pub struct StopOutcome {
    pub removed_observations: usize,
    /// Exit is already confirmed when one of these best-effort retirements fails.
    pub cleanup_issues: Vec<String>,
}

#[derive(Debug, Default)]
pub struct RenameOutcome {
    /// A stopped name is authoritative even if its regenerable history cache
    /// cannot be updated. A live host owns its own history reconciliation.
    pub cache_issue: Option<String>,
}

pub struct ControlSnapshot {
    history: HistorySnapshot,
    // A begun stopped-name transaction must outlive caller cancellation and
    // retain its recovery handles until retry or owner shutdown.
    pending_name: Option<StoppedNameEdit>,
}

impl ControlSnapshot {
    /// A complete, read-only observation of the captured namespace. An error
    /// never becomes an empty list or authority to mutate remembered history.
    pub async fn observe(
        scope: &DiscoveryScope,
        configured_state: &Path,
        include_hidden: bool,
    ) -> Result<Self> {
        Self::observe_at(scope, None, configured_state, include_hidden).await
    }

    /// The service supplies its captured current ready location; selector-only
    /// CLI discovery deliberately supplies none.
    pub async fn observe_at(
        scope: &DiscoveryScope,
        current: Option<&KnownReadLocation>,
        configured_state: &Path,
        include_hidden: bool,
    ) -> Result<Self> {
        let history = windows_catalog::snapshot_with_history_in_scope(
            scope,
            current,
            configured_state,
            include_hidden,
        )
        .await?;
        Ok(Self {
            history,
            pending_name: None,
        })
    }

    pub fn history(&self) -> &HistorySnapshot {
        &self.history
    }

    #[cfg(test)]
    pub(crate) fn retain_pending_name_for_test(&mut self, edit: StoppedNameEdit) {
        assert!(edit.recovery_pending());
        self.pending_name = Some(edit);
    }

    pub fn select(&self, target: UserSelector<'_>) -> Result<usize> {
        self.history
            .select_index(target.selector, target.working_directory)?
            .with_context(|| {
                format!(
                    "no session matches {}; use --session-list to see available sessions",
                    target.selector.display()
                )
            })
    }

    /// Stop precisely this snapshot's live publication. Metadata is passed to
    /// a new authenticated connection, and its returned receipt must witness
    /// actual process exit before any retained observation is retired.
    pub async fn stop(&self, index: usize, force: bool) -> Result<StopOutcome> {
        let HistoryTarget::Live { publication, .. } = self
            .history
            .target(index)
            .context("selected session is unavailable")?
        else {
            anyhow::bail!("selected session is already stopped");
        };
        let metadata = publication.metadata();
        let receipt = if metadata.protocol == crate::protocol::VERSION {
            let receipt = if force {
                force_shutdown_host(metadata).await?
            } else {
                shutdown_host(metadata).await?
            };
            await_host_stopped(&receipt).await?;
            receipt
        } else {
            ensure!(
                force,
                "persistent session process {} speaks incompatible protocol {}; use --session-stop --force to terminate it",
                metadata.process.pid,
                metadata.protocol
            );
            // This path is selected only by the retained incompatible metadata.
            // The lifecycle operation authenticates the actual pipe peer again.
            terminate_incompatible_host(&publication.observations()[0]).await?
        };
        let mut outcome = StopOutcome::default();
        for candidate in publication.observations() {
            // Scope-based ready observations have Source::UnscopedReady. They
            // cannot authorize retirement; the host's own teardown handles its
            // ready file. Registry candidates retain exact cleanup authority.
            if candidate.origin() == CandidateOrigin::ConfiguredReady {
                continue;
            }
            match remove_stopped_observation(&receipt, candidate) {
                Ok(Removal::Removed) => outcome.removed_observations += 1,
                Ok(Removal::Missing) => {}
                Ok(other) => outcome.cleanup_issues.push(bounded(format!(
                    "observed {:?} record was not retired: {other:?}",
                    candidate.origin()
                ))),
                Err(error) => outcome.cleanup_issues.push(bounded(format!(
                    "observed registry record could not be retired: {error:#}"
                ))),
            }
        }
        Ok(outcome)
    }

    /// A disconnected live rename may have reached the host. It never grants
    /// permission to write a local name file or history entry as a fallback.
    pub async fn rename(&mut self, index: usize, name: &str) -> Result<RenameOutcome> {
        ensure!(
            self.pending_name.is_none(),
            "a stopped-name update still requires recovery"
        );
        match self
            .history
            .target(index)
            .context("selected session is unavailable")?
        {
            HistoryTarget::Live { publication, .. } => {
                ensure!(
                    publication.metadata().protocol == crate::protocol::VERSION,
                    "incompatible persistent session cannot be renamed"
                );
                rename_host(publication.metadata(), name)
                    .await
                    .context("live rename outcome may be unknown after the request was sent")?;
                Ok(RenameOutcome::default())
            }
            HistoryTarget::Stopped { .. } => {
                self.pending_name = Some(self.history.stopped_name_editor(index)?);
                let result = self
                    .pending_name
                    .as_mut()
                    .expect("edit installed")
                    .rename(name);
                if !self
                    .pending_name
                    .as_ref()
                    .expect("edit installed")
                    .recovery_pending()
                {
                    self.pending_name = None;
                }
                Ok(RenameOutcome {
                    cache_issue: result?.cache_error,
                })
            }
        }
    }

    pub fn recovery_pending(&self) -> bool {
        self.pending_name.is_some()
    }

    pub fn retry_name_recovery(&mut self) -> Result<()> {
        if let Some(edit) = &mut self.pending_name {
            edit.retry_recovery()?;
            self.pending_name = None;
        }
        Ok(())
    }

    /// Forget only unchanged, proven stopped history rows from this complete
    /// observation. Live records and indeterminate observations are preserved.
    pub fn clean(&self) -> Result<usize> {
        ensure!(
            self.pending_name.is_none(),
            "a stopped-name update still requires recovery"
        );
        self.history.clear_stopped()
    }

    /// The caller must retain this owner until completion. If its future is
    /// cancelled, call `run_to_completion` again: the admitted result is then
    /// reported unknown and the remaining publications are still attempted.
    pub fn stop_all(&self, force: bool) -> Result<StopAllOperation<'_>> {
        ensure!(
            self.pending_name.is_none(),
            "a stopped-name update still requires recovery"
        );
        let live = self
            .history
            .entries()
            .iter()
            .enumerate()
            .filter_map(|(index, entry)| entry.row().running.then_some(index))
            .collect::<Vec<_>>();
        Ok(StopAllOperation {
            snapshot: self,
            force,
            total: live.len(),
            live,
            next: 0,
            admitted: None,
            stopped: 0,
            failures: Vec::new(),
            failure_bytes: 0,
            failed: 0,
            omitted_failure_details: 0,
            unknown: 0,
            cleanup_issues: Vec::new(),
            cleanup_issue_count: 0,
            omitted_cleanup_details: 0,
        })
    }
}

pub struct StopAllOperation<'a> {
    snapshot: &'a ControlSnapshot,
    force: bool,
    live: Vec<usize>,
    next: usize,
    admitted: Option<usize>,
    total: usize,
    stopped: usize,
    failures: Vec<String>,
    failure_bytes: usize,
    failed: usize,
    omitted_failure_details: usize,
    unknown: usize,
    cleanup_issues: Vec<String>,
    cleanup_issue_count: usize,
    omitted_cleanup_details: usize,
}

#[derive(Debug)]
pub struct StopAllReport {
    pub stopped: usize,
    pub total: usize,
    pub failures: Vec<String>,
    pub failed: usize,
    pub omitted_failure_details: usize,
    pub unknown: usize,
    pub admitted_summary: Option<String>,
    pub cleanup_issues: Vec<String>,
    pub cleanup_issue_count: usize,
    pub omitted_cleanup_details: usize,
}

impl StopAllOperation<'_> {
    /// Sequentially attempts every retained live publication. A dropped
    /// invocation leaves `admitted` in this owner, never silently retries it.
    pub async fn run_to_completion(&mut self) {
        if let Some(index) = self.admitted.take() {
            self.unknown += 1;
            self.record_failure(index, "stop outcome unknown after cancellation");
        }
        while let Some(&index) = self.live.get(self.next) {
            self.next += 1;
            self.admitted = Some(index);
            let result = self.snapshot.stop(index, self.force).await;
            self.admitted = None;
            match result {
                Ok(outcome) => {
                    self.stopped += 1;
                    for issue in outcome.cleanup_issues {
                        self.record_cleanup_issue(index, &issue);
                    }
                }
                Err(error) => {
                    self.failed += 1;
                    self.record_failure(index, &format!("{error:#}"));
                }
            }
        }
    }

    pub fn report(&self) -> StopAllReport {
        StopAllReport {
            stopped: self.stopped,
            total: self.total,
            failures: self.failures.clone(),
            failed: self.failed,
            omitted_failure_details: self.omitted_failure_details,
            unknown: self.unknown + usize::from(self.admitted.is_some()),
            admitted_summary: self.admitted.map(|index| self.row_summary(index)),
            cleanup_issues: self.cleanup_issues.clone(),
            cleanup_issue_count: self.cleanup_issue_count,
            omitted_cleanup_details: self.omitted_cleanup_details,
        }
    }

    fn row_summary(&self, index: usize) -> String {
        let row = self.snapshot.history.entries()[index].row();
        bounded(format!(
            "{} {} ({})",
            self.publication_identity(index),
            row.display_name(),
            row.project_root.display()
        ))
    }

    fn publication_identity(&self, index: usize) -> String {
        let publication = match self.snapshot.history.target(index) {
            Some(HistoryTarget::Live { publication, .. }) => publication,
            _ => unreachable!("stop-all indices are live publications"),
        };
        let metadata = publication.metadata();
        bounded_to(
            format!(
                "pid={} created={} incarnation={} pipe={}",
                metadata.process.pid,
                metadata.process.creation_time,
                metadata.incarnation,
                metadata.address.as_str(),
            ),
            240,
        )
    }

    fn record_failure(&mut self, index: usize, reason: &str) {
        let summary = format!(
            "{}: {}",
            self.publication_identity(index),
            bounded_to(reason.to_owned(), 260)
        );
        if self.failure_bytes + summary.len() <= MAX_STOP_ALL_FAILURE_BYTES {
            self.failure_bytes += summary.len();
            self.failures.push(summary);
        } else {
            self.omitted_failure_details += 1;
        }
    }

    fn record_cleanup_issue(&mut self, index: usize, reason: &str) {
        self.cleanup_issue_count += 1;
        let summary = format!(
            "{}: {}",
            self.publication_identity(index),
            bounded_to(reason.to_owned(), 260)
        );
        if self.failure_bytes + summary.len() <= MAX_STOP_ALL_FAILURE_BYTES {
            self.failure_bytes += summary.len();
            self.cleanup_issues.push(summary);
        } else {
            self.omitted_cleanup_details += 1;
        }
    }
}

fn bounded(value: String) -> String {
    bounded_to(value, MAX_FAILURE_TEXT_BYTES)
}

fn bounded_to(mut value: String, limit: usize) -> String {
    if value.len() > limit {
        let mut end = limit - 3;
        while !value.is_char_boundary(end) {
            end -= 1;
        }
        value.truncate(end);
        value.push_str("...");
    }
    value
}

#[cfg(test)]
mod tests;
