// SPDX-License-Identifier: MPL-2.0

//! Capture a branch row for the ordered, asynchronous fetch worker.

use super::App;
use crate::git::{FetchBranchTarget, GitMutation, GitOperation};

impl App {
    pub(super) fn selected_fetch_target(&self) -> Result<FetchBranchTarget, String> {
        if !self.active_buffer().is_git_branches() {
            return Err("open the branch list and select a branch to fetch".to_owned());
        }
        let row = self.active_buffer().offset_to_row(self.active().head());
        let row = self
            .git_state
            .branch_rows()
            .get(row)
            .ok_or_else(|| "this row is not a branch".to_owned())?;
        if let Some(remote) = &row.remote {
            return Ok(FetchBranchTarget::RemoteTrackingRef(
                remote.reference.clone(),
            ));
        }
        let branch = row
            .branch
            .as_ref()
            .ok_or_else(|| "this row is not a branch".to_owned())?;
        let upstream = branch
            .upstream
            .as_ref()
            .ok_or_else(|| "configure a remote upstream for this local branch first".to_owned())?;
        if upstream.remote == "." {
            return Err("a local '.' upstream is not a network fetch target".to_owned());
        }
        if upstream.remote.is_empty() || !upstream.tracking_reference.starts_with("refs/remotes/") {
            return Err("the configured upstream must map to a remote-tracking ref".to_owned());
        }
        Ok(FetchBranchTarget::LocalBranch(branch.name.clone()))
    }

    pub(super) fn fetch_selected_branch(&mut self) {
        let target = match self.selected_fetch_target() {
            Ok(target) => target,
            Err(reason) => {
                self.action_failed(reason);
                return;
            }
        };
        let Some(repository) = self.git.repository().cloned() else {
            self.action_failed("this project is not in a Git repository");
            return;
        };
        if self.ports.git_service.is_some() {
            let mut refresh = self.git_refresh_spec(&repository);
            refresh.branches = true;
            let _ = self.request_git(GitOperation::Mutate {
                repository,
                mutation: GitMutation::FetchBranch(target),
                refresh,
            });
            return;
        }
        // Isolated tests may use a synchronous provider; production always queues.
        let Some(provider) = self.ports.git.as_deref() else {
            self.action_failed("no `git` executable was found");
            return;
        };
        let result = provider.fetch_branch(&repository, &target);
        self.refresh_git();
        self.refresh_git_status_buffer();
        self.refresh_git_branches_buffer("");
        match result {
            Ok(summary) => self.status(summary.lines().next().unwrap_or("fetched selected branch")),
            Err(error) => self.error_from("Git", "Git fetch failed", error.to_string()),
        }
    }
}
