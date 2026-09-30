// SPDX-License-Identifier: MPL-2.0

use super::*;
use crate::git::merge::*;
use crate::git::{BufferRevisionGuard, ComparisonTarget};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Mutex, OnceLock};

fn refusal(detail: impl Into<String>) -> GitError {
    GitError::Malformed {
        command: "reviewed merge".into(),
        detail: detail.into(),
    }
}
fn stale() -> GitError {
    refusal("repository, refs, index, disk, settings or buffer changed; review again")
}

impl GitCliProvider {
    fn merge_read(&self, repo: &Repository, args: &[&str]) -> Result<Vec<u8>> {
        self.run_read_bounded(repo.workdir(), args, self.max_output_bytes)
    }
    fn merge_tip(&self, repo: &Repository, reference: &str) -> Result<String> {
        let bytes = self.merge_read(
            repo,
            &[
                "rev-parse",
                "--verify",
                "--end-of-options",
                &format!("{reference}^{{commit}}"),
            ],
        )?;
        let oid = String::from_utf8_lossy(&bytes).trim().to_owned();
        if !valid_object_id(&oid) {
            return Err(refusal("invalid commit identity"));
        }
        Ok(oid)
    }
    pub(super) fn inspect_operation(&self, repo: &Repository) -> Result<RepositoryOperation> {
        // Markers belong to git_dir, not the directory shared by linked worktrees.
        for (marker, state) in [
            ("rebase-merge", RepositoryOperation::Rebase),
            ("rebase-apply", RepositoryOperation::Rebase),
            ("CHERRY_PICK_HEAD", RepositoryOperation::CherryPick),
            ("REVERT_HEAD", RepositoryOperation::Revert),
            ("BISECT_LOG", RepositoryOperation::Bisect),
            ("sequencer", RepositoryOperation::Other),
        ] {
            if repo
                .git_dir()
                .join(marker)
                .try_exists()
                .map_err(|e| refusal(e.to_string()))?
            {
                return Ok(state);
            }
        }
        let path = repo.git_dir().join("MERGE_HEAD");
        if !path.try_exists().map_err(|e| refusal(e.to_string()))? {
            return Ok(RepositoryOperation::Idle);
        }
        let bytes = read_file_for_comparison(&path, MAX_GIT_MARKER_BYTES)?;
        let text = std::str::from_utf8(&bytes).map_err(|_| refusal("invalid MERGE_HEAD"))?;
        let parents: Vec<_> = text.lines().map(str::to_owned).collect();
        if parents.is_empty() || parents.iter().any(|oid| !valid_object_id(oid)) {
            return Err(refusal("invalid MERGE_HEAD"));
        }
        Ok(RepositoryOperation::Merge { parents })
    }
    pub(super) fn read_conflicts(&self, repo: &Repository) -> Result<ConflictInventory> {
        let index = self.merge_read(repo, &["ls-files", "--stage", "-z"])?;
        let mut entries: Vec<ConflictEntry> = parse_stages(&index)?
            .into_iter()
            .filter(|entry| {
                entry.base.is_some() || entry.current.is_some() || entry.other.is_some()
            })
            .collect();
        if !entries.is_empty() {
            let mut paths = Vec::new();
            for entry in &entries {
                paths.extend_from_slice(entry.path.as_os_str().as_encoded_bytes());
                paths.push(0);
            }
            let attributes = self.run_with_input_bounded(
                repo.workdir(),
                &["check-attr", "-z", "conflict-marker-size", "--stdin"],
                &paths,
                self.max_output_bytes,
            )?;
            let fields: Vec<_> = attributes
                .split(|b| *b == 0)
                .filter(|v| !v.is_empty())
                .collect();
            if fields.len() != entries.len() * 3 {
                return Err(refusal("invalid marker attribute response"));
            }
            for (entry, attribute) in entries.iter_mut().zip(fields.chunks_exact(3)) {
                let width = std::str::from_utf8(attribute[2])
                    .ok()
                    .and_then(|v| v.parse::<usize>().ok())
                    .unwrap_or(7);
                if !(1..=1024).contains(&width) {
                    return Err(refusal("unsupported conflict marker width"));
                }
                entry.marker_width = width;
            }
        }
        let operation = self.inspect_operation(repo)?;
        let (current_identity, other_identity) = match &operation {
            RepositoryOperation::Merge { parents } => (
                format!("Current: {}", self.status(repo)?.head.label()),
                format!("Other: {}", parents.join(", ")),
            ),
            RepositoryOperation::Rebase => (
                "Onto checkout (stage 2)".into(),
                "Replayed commit (stage 3)".into(),
            ),
            _ => ("Stage 2".into(), "Stage 3".into()),
        };
        Ok(ConflictInventory {
            current_identity,
            other_identity,
            operation: self.inspect_operation(repo)?,
            head_oid: self.head_oid(repo)?,
            entries,
            index_identity: crate::hash::sha256_hex(&index),
        })
    }
    fn merge_settings(&self, repo: &Repository) -> Result<String> {
        let config = self.merge_read(repo, &["config", "--null", "--list"])?;
        for item in config.split(|b| *b == 0).filter(|v| !v.is_empty()) {
            let key = item.split(|b| *b == b'\n').next().unwrap_or_default();
            let key = String::from_utf8_lossy(key).to_lowercase();
            if key.starts_with("merge.")
                && !matches!(
                    key.as_str(),
                    "merge.ff"
                        | "merge.log"
                        | "merge.stat"
                        | "merge.verbosity"
                        | "merge.autostash"
                        | "merge.verifysignatures"
                        | "merge.suppressdest"
                        | "merge.conflictstyle"
                        | "merge.renames"
                        | "merge.renamelimit"
                        | "merge.directoryrenames"
                )
                || key.ends_with(".mergeoptions")
                || key == "core.sparsecheckout"
                || key == "core.attributesfile"
            {
                return Err(refusal(format!(
                    "reviewed merge does not support configuration {key}; use Git directly"
                )));
            }
        }
        // Worktree-only and global attributes are not represented by merge-tree.
        for path in [
            repo.git_dir().join("info/attributes"),
            repo.common_dir().join("info/attributes"),
        ] {
            if path.try_exists().map_err(|e| refusal(e.to_string()))? {
                return Err(refusal("reviewed merge does not support info/attributes"));
            }
        }
        Ok(crate::hash::sha256_hex(&config))
    }
    fn merge_disk(&self, repo: &Repository) -> Result<String> {
        let tracked = self.merge_read(
            repo,
            &[
                "ls-files",
                "-z",
                "--cached",
                "--others",
                "--exclude-standard",
            ],
        )?;
        let mut paths = BTreeSet::new();
        for bytes in tracked.split(|b| *b == 0).filter(|v| !v.is_empty()) {
            paths.insert(status::path_from_bytes(bytes).map_err(refusal)?);
        }
        let mut identity = Vec::new();
        let mut budget = 4u64 * 1024 * 1024 * 1024;
        let started = std::time::Instant::now();
        for path in paths {
            if self.is_cancelled() {
                return Err(GitError::Cancelled {
                    command: "merge disk baseline".into(),
                });
            }
            if started.elapsed() >= self.local_read_timeout {
                return Err(GitError::TimedOut {
                    command: "merge disk baseline".into(),
                    seconds: self.local_read_timeout.as_secs(),
                });
            }
            let absolute = repo.workdir().join(&path);
            self.relative(repo, &absolute)?;
            let mut parent = absolute.parent();
            while let Some(directory) = parent {
                if directory == repo.workdir() {
                    break;
                }
                if std::fs::symlink_metadata(directory).is_ok_and(|m| m.file_type().is_symlink()) {
                    return Err(refusal("a working path traverses a symlink"));
                }
                parent = directory.parent();
            }
            identity.extend_from_slice(path.as_os_str().as_encoded_bytes());
            identity.push(0);
            match std::fs::symlink_metadata(&absolute) {
                Err(e) if e.kind() == io::ErrorKind::NotFound => identity.push(b'D'),
                Err(e) => return Err(refusal(e.to_string())),
                Ok(metadata) if metadata.file_type().is_symlink() => {
                    identity.push(b'L');
                    let target =
                        std::fs::read_link(&absolute).map_err(|e| refusal(e.to_string()))?;
                    identity.extend_from_slice(target.as_os_str().as_encoded_bytes());
                }
                Ok(metadata) if metadata.is_file() => {
                    if metadata.len() > budget {
                        return Err(GitError::TooLarge {
                            command: "merge disk baseline".into(),
                            limit: 4usize * 1024 * 1024 * 1024,
                        });
                    }

                    let mut file =
                        std::fs::File::open(&absolute).map_err(|e| refusal(e.to_string()))?;
                    let mut buffer = [0u8; 65536];
                    let mut content_identity = String::new();
                    loop {
                        if self.is_cancelled() {
                            return Err(GitError::Cancelled {
                                command: "merge disk baseline".into(),
                            });
                        }
                        if started.elapsed() >= self.local_read_timeout {
                            return Err(GitError::TimedOut {
                                command: "merge disk baseline".into(),
                                seconds: self.local_read_timeout.as_secs(),
                            });
                        }
                        let count = file.read(&mut buffer).map_err(|e| refusal(e.to_string()))?;
                        if count == 0 {
                            break;
                        }
                        if count as u64 > budget {
                            return Err(GitError::TooLarge {
                                command: "merge disk baseline".into(),
                                limit: 4usize * 1024 * 1024 * 1024,
                            });
                        }
                        budget -= count as u64;
                        let chunk = crate::hash::sha256_hex(&buffer[..count]);
                        content_identity = crate::hash::sha256_hex(
                            format!("{content_identity}{chunk}").as_bytes(),
                        );
                    }
                    identity.push(b'F');
                    identity.extend_from_slice(content_identity.as_bytes());
                    identity.push(u8::from(metadata.permissions().readonly()));
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::PermissionsExt;
                        identity.extend_from_slice(&metadata.permissions().mode().to_le_bytes());
                    }
                }
                Ok(_) => identity.push(b'O'),
            }
            identity.push(0);
        }
        Ok(crate::hash::sha256_hex(&identity))
    }
    fn reject_tree_attributes(&self, repo: &Repository, oid: &str) -> Result<()> {
        let paths = self.merge_read(repo, &["ls-tree", "-rz", "--name-only", oid])?;
        let attributes = self.run_with_input_bounded(
            repo.workdir(),
            &[
                "check-attr",
                &format!("--source={oid}"),
                "-z",
                "--all",
                "--stdin",
            ],
            &paths,
            self.max_output_bytes,
        )?;
        let fields: Vec<_> = attributes
            .split(|b| *b == 0)
            .filter(|v| !v.is_empty())
            .collect();
        if fields.len() % 3 != 0 {
            return Err(refusal("invalid attribute output"));
        }
        for triple in fields.chunks_exact(3) {
            if triple[1] == b"filter"
                || triple[1] == b"working-tree-encoding"
                || triple[1] == b"merge"
                    && !matches!(
                        triple[2],
                        b"text" | b"binary" | b"union" | b"set" | b"unset"
                    )
            {
                return Err(refusal(
                    "reviewed merging does not support custom merge drivers, filters, or working-tree-encoding attributes",
                ));
            }
        }
        Ok(())
    }
    fn probe_merge_tree(&self, repo: &Repository, oid: &str) -> Result<()> {
        static CAPABILITIES: OnceLock<Mutex<BTreeSet<PathBuf>>> = OnceLock::new();
        let key = self.program.clone();
        if CAPABILITIES
            .get_or_init(Mutex::default)
            .lock()
            .unwrap()
            .contains(&key)
        {
            return Ok(());
        }
        let (_, bytes) = self.run_bounded_until(repo.workdir(), &["merge-tree", "--write-tree", "-z", "--messages", oid, oid], self.max_output_bytes, self.local_read_timeout, true).map_err(|e| match e {
            GitError::Failed { .. } => refusal("Git lacks the required merge-tree --write-tree -z --messages capability; upgrade Git"),
            other => other,
        })?;
        let _ = parse_preview(&bytes, true)?;
        CAPABILITIES.get().unwrap().lock().unwrap().insert(key);
        Ok(())
    }
    pub(super) fn prepare_reviewed_merge(
        &self,
        repo: &Repository,
        source: &str,
        guard: BufferRevisionGuard,
    ) -> Result<MergePlan> {
        if !guard.is_valid() {
            return Err(stale());
        }
        if !source.starts_with("refs/heads/") && !source.starts_with("refs/remotes/") {
            return Err(refusal("select a full local or remote-tracking branch ref"));
        }
        self.merge_read(repo, &["check-ref-format", source])?;
        if self.inspect_operation(repo)? != RepositoryOperation::Idle {
            return Err(refusal(
                "finish the active repository operation before merging",
            ));
        }
        let status = self.status(repo)?;
        if !status.files.is_empty() {
            return Err(GitError::DirtyWorktree {
                files: status.files.len(),
            });
        }
        let Head::Branch(branch) = status.head else {
            return Err(refusal(
                "reviewed merging requires a current committed local branch",
            ));
        };
        let destination_reference = format!("refs/heads/{branch}");
        let destination_oid = self.merge_tip(repo, &destination_reference)?;
        let source_oid = self.merge_tip(repo, source)?;
        let settings_identity = self.merge_settings(repo)?;
        self.reject_tree_attributes(repo, &destination_oid)?;
        self.reject_tree_attributes(repo, &source_oid)?;
        let flags = self.merge_read(repo, &["ls-files", "-v", "-z"])?;
        if flags.split(|b| *b == 0).any(|line| {
            line.first()
                .is_some_and(|flag| flag.is_ascii_lowercase() || *flag == b'S')
        }) {
            return Err(refusal(
                "reviewed merging does not support assume-unchanged or sparse/skip-worktree files",
            ));
        }
        let fingerprint = self.repository_fingerprint(repo)?;
        let disk_identity = self.merge_disk(repo)?;
        self.probe_merge_tree(repo, &destination_oid)?;
        let outcome = if self.merge_ancestor(repo, &source_oid, &destination_oid)? {
            MergePreviewOutcome::AlreadyContained
        } else if self.merge_ancestor(repo, &destination_oid, &source_oid)? {
            MergePreviewOutcome::FastForward
        } else {
            let (clean, bytes) = self.run_bounded_until(
                repo.workdir(),
                &[
                    "merge-tree",
                    "--write-tree",
                    "-z",
                    "--messages",
                    &destination_oid,
                    &source_oid,
                ],
                self.max_output_bytes,
                self.local_read_timeout,
                true,
            )?;
            parse_preview(&bytes, clean)?
        };
        let result_oid = match &outcome {
            MergePreviewOutcome::Merge { tree_oid, .. } => tree_oid.clone(),
            MergePreviewOutcome::FastForward => source_oid.clone(),
            MergePreviewOutcome::AlreadyContained => destination_oid.clone(),
        };
        let comparison = self.read_tree_comparison(
            repo,
            ComparisonTarget::Branch {
                reference: source.into(),
                label: source.into(),
            },
            destination_reference.clone(),
            "Reviewed result".into(),
            destination_oid.clone(),
            result_oid,
        )?;
        let plan = MergePlan {
            comparison,
            repository: repo.clone(),
            destination_reference,
            destination_oid,
            source_reference: source.into(),
            source_oid,
            outcome,
            fingerprint,
            disk_identity,
            settings_identity,
            guard,
        };
        self.check_merge_plan(repo, &plan)?;
        Ok(plan)
    }
    fn merge_ancestor(&self, repo: &Repository, a: &str, b: &str) -> Result<bool> {
        self.run_bounded_until(
            repo.workdir(),
            &["merge-base", "--is-ancestor", a, b],
            self.max_output_bytes,
            self.local_read_timeout,
            true,
        )
        .map(|(success, _)| success)
    }
    fn check_merge_plan(&self, repo: &Repository, plan: &MergePlan) -> Result<()> {
        if repo != &plan.repository
            || !plan.guard.is_valid()
            || self.inspect_operation(repo)? != RepositoryOperation::Idle
            || self.repository_fingerprint(repo)? != plan.fingerprint
            || self.merge_disk(repo)? != plan.disk_identity
            || self.merge_settings(repo)? != plan.settings_identity
            || self.merge_tip(repo, &plan.source_reference)? != plan.source_oid
            || self.merge_tip(repo, &plan.destination_reference)? != plan.destination_oid
        {
            return Err(stale());
        }
        if !self.status(repo)?.files.is_empty() {
            return Err(stale());
        }
        let branch = plan
            .destination_reference
            .strip_prefix("refs/heads/")
            .unwrap_or_default();
        if self.status(repo)?.head != Head::Branch(branch.into()) {
            return Err(stale());
        }
        Ok(())
    }
    pub(super) fn apply_reviewed_merge(
        &self,
        repo: &Repository,
        plan: &MergePlan,
    ) -> Result<MergeApplyResult> {
        self.check_merge_plan(repo, plan)?;
        if matches!(plan.outcome, MergePreviewOutcome::AlreadyContained) {
            return Ok(MergeApplyResult {
                diagnostic: None,
                outcome: MergeApplied::AlreadyContained,
                inventory: self.read_conflicts(repo)?,
            });
        }
        if !plan.guard.is_valid() {
            return Err(stale());
        }
        let policy = if matches!(plan.outcome, MergePreviewOutcome::FastForward) {
            "--ff-only"
        } else {
            "--no-ff"
        };
        let outcome = self.run(
            repo.workdir(),
            &[
                "-c",
                "rerere.enabled=false",
                "-c",
                "rerere.autoupdate=false",
                "merge",
                "--strategy=ort",
                policy,
                "--no-commit",
                "--no-autostash",
                "--no-edit",
                "--",
                &plan.source_oid,
            ],
        );
        let inventory = self.uncancellable().read_conflicts(repo)?;
        // A conflict is a successful state transition only if the actual
        // worktree operation and stages prove it, and the process exited 1.
        if matches!(&outcome, Err(GitError::Failed { code: Some(1), .. }))
            && inventory.operation
                == (RepositoryOperation::Merge {
                    parents: vec![plan.source_oid.clone()],
                })
        {
            return Ok(MergeApplyResult {
                diagnostic: match &outcome {
                    Err(GitError::Failed { stderr, .. }) => Some(stderr.clone()),
                    _ => None,
                },
                outcome: MergeApplied::Conflicted,
                inventory,
            });
        }
        outcome?;
        let applied = match inventory.operation {
            RepositoryOperation::Merge { .. } => MergeApplied::PendingCommit,
            RepositoryOperation::Idle
                if inventory.head_oid.as_deref() == Some(&plan.source_oid) =>
            {
                MergeApplied::FastForward
            }
            _ => {
                return Err(refusal(
                    "merge ended in an unexpected repository state; inspect Git status",
                ));
            }
        };
        Ok(MergeApplyResult {
            diagnostic: None,
            outcome: applied,
            inventory,
        })
    }
    pub(super) fn review_resolution(
        &self,
        repo: &Repository,
        path: &Path,
        choice: ResolutionChoice,
        guard: BufferRevisionGuard,
    ) -> Result<ResolutionPlan> {
        self.relative(repo, path)?;
        let inventory = self.read_conflicts(repo)?;
        let relative = self.relative(repo, path)?;
        let entry = inventory
            .entries
            .iter()
            .find(|entry| entry.path == relative)
            .cloned()
            .ok_or_else(|| refusal("path is no longer unresolved"))?;
        if !guard.is_valid() {
            return Err(stale());
        }
        if !matches!(choice, ResolutionChoice::SavedFile { .. })
            && ((entry.current.is_none() || entry.other.is_none())
                && inventory.entries.iter().any(|e| {
                    e.path != entry.path
                        && e.base.is_some()
                        && (e.current.is_none() || e.other.is_none())
                })
                || entry
                    .current
                    .as_ref()
                    .is_some_and(|s| !matches!(s.mode.as_str(), "100644" | "100755"))
                || entry
                    .other
                    .as_ref()
                    .is_some_and(|s| !matches!(s.mode.as_str(), "100644" | "100755")))
        {
            return Err(refusal(
                "this structural conflict requires manual file management; review and stage each affected path",
            ));
        }
        if matches!(
            choice,
            ResolutionChoice::SavedFile {
                allow_literal_markers: false
            }
        ) {
            match std::fs::symlink_metadata(path) {
                Err(e) if e.kind() == io::ErrorKind::NotFound => (),
                Err(e) => return Err(refusal(e.to_string())),
                Ok(metadata) if metadata.is_file() => {
                    let bytes = read_file_for_comparison(path, self.max_output_bytes)?;
                    if marker_like(&bytes, entry.marker_width) {
                        return Err(refusal(
                            "file contains conflict markers; resolve them or explicitly review literal marker text",
                        ));
                    }
                }
                Ok(metadata) if metadata.file_type().is_symlink() => (),
                Ok(_) => {
                    return Err(refusal(
                        "structural path requires explicit manual file management",
                    ));
                }
            }
        }
        let disk_identity = self.merge_disk(repo)?;
        Ok(ResolutionPlan {
            repository: repo.clone(),
            path: path.to_owned(),
            choice,
            entry,
            inventory,
            disk_identity,
            guard,
        })
    }
    pub(super) fn apply_resolution(
        &self,
        repo: &Repository,
        plan: &ResolutionPlan,
    ) -> Result<ConflictInventory> {
        if repo != &plan.repository
            || !plan.guard.is_valid()
            || self.read_conflicts(repo)? != plan.inventory
            || self.merge_disk(repo)? != plan.disk_identity
        {
            return Err(stale());
        }
        self.relative(repo, &plan.path)?;
        if !plan.guard.is_valid() {
            return Err(stale());
        }
        match plan.choice {
            ResolutionChoice::SavedFile { .. } => (),
            ResolutionChoice::Current | ResolutionChoice::Other => {
                let selected = if plan.choice == ResolutionChoice::Current {
                    &plan.entry.current
                } else {
                    &plan.entry.other
                };
                let relative = self.relative(repo, &plan.path)?;
                if selected.is_none() {
                    // Git owns path validation and refuses an unexpected directory.
                    self.run(
                        repo.workdir(),
                        &[
                            OsStr::new("rm"),
                            OsStr::new("-f"),
                            OsStr::new("--"),
                            relative.as_os_str(),
                        ],
                    )?;
                } else {
                    let side = if plan.choice == ResolutionChoice::Current {
                        "--ours"
                    } else {
                        "--theirs"
                    };
                    self.run(
                        repo.workdir(),
                        &[
                            OsStr::new("checkout"),
                            OsStr::new(side),
                            OsStr::new("--"),
                            relative.as_os_str(),
                        ],
                    )?;
                }
            }
        }
        if self
            .read_conflicts(repo)?
            .entries
            .iter()
            .any(|entry| entry.path == plan.entry.path)
        {
            self.stage(repo, &plan.path)?;
        }
        let inventory = self.read_conflicts(repo)?;
        if inventory
            .entries
            .iter()
            .any(|entry| entry.path == plan.entry.path)
        {
            return Err(refusal(
                "Git retained unresolved stages for the reviewed path",
            ));
        }
        Ok(inventory)
    }
    pub(super) fn review_completion(
        &self,
        repo: &Repository,
        guard: BufferRevisionGuard,
    ) -> Result<MergeCompletionPlan> {
        let inventory = self.read_conflicts(repo)?;
        if !matches!(inventory.operation, RepositoryOperation::Merge { .. }) {
            return Err(refusal("there is no active merge to continue or abort"));
        }
        if !guard.is_valid() {
            return Err(stale());
        }
        let message_path = repo.git_dir().join("MERGE_MSG");
        let message = String::from_utf8(read_file_for_comparison(
            &message_path,
            MAX_GIT_MARKER_BYTES,
        )?)
        .map_err(|_| refusal("merge message is not UTF-8"))?;
        let staged_diff = self.diff(repo, DiffScope::Staged, None)?;
        Ok(MergeCompletionPlan {
            repository: repo.clone(),
            inventory,
            message,
            staged_diff,
            disk_identity: self.merge_disk(repo)?,
            markers_identity: self.completion_markers(repo)?,
            settings_identity: crate::hash::sha256_hex(
                &self.merge_read(repo, &["config", "--null", "--list"])?,
            ),
            guard,
        })
    }
    fn completion_markers(&self, repo: &Repository) -> Result<String> {
        let mut contents = Vec::new();
        for name in ["ORIG_HEAD", "MERGE_AUTOSTASH", "MERGE_MSG", "MERGE_MODE"] {
            let path = repo.git_dir().join(name);
            if path.try_exists().map_err(|e| refusal(e.to_string()))? {
                contents.extend_from_slice(name.as_bytes());
                contents.extend_from_slice(&read_file_for_comparison(&path, MAX_GIT_MARKER_BYTES)?);
            }
            contents.push(0);
        }
        Ok(crate::hash::sha256_hex(&contents))
    }
    fn check_completion(&self, repo: &Repository, plan: &MergeCompletionPlan) -> Result<()> {
        if repo != &plan.repository
            || !plan.guard.is_valid()
            || self.read_conflicts(repo)? != plan.inventory
            || self.merge_disk(repo)? != plan.disk_identity
            || self.completion_markers(repo)? != plan.markers_identity
            || crate::hash::sha256_hex(&self.merge_read(repo, &["config", "--null", "--list"])?)
                != plan.settings_identity
        {
            return Err(stale());
        }
        Ok(())
    }
    pub(super) fn finish_merge(
        &self,
        repo: &Repository,
        plan: &MergeCompletionPlan,
        message: &str,
    ) -> Result<String> {
        self.check_completion(repo, plan)?;
        if !plan.inventory.entries.is_empty() {
            return Err(refusal(
                "resolve all index conflicts before continuing the merge",
            ));
        }
        if !plan.guard.is_valid() {
            return Err(stale());
        }
        self.commit(repo, message)
    }
    pub(super) fn cancel_merge(
        &self,
        repo: &Repository,
        plan: &MergeCompletionPlan,
    ) -> Result<String> {
        self.check_completion(repo, plan)?;
        if !plan.guard.is_valid() {
            return Err(stale());
        }
        self.run_text(repo.workdir(), &["merge", "--abort"])
    }
}

fn marker_like(bytes: &[u8], width: usize) -> bool {
    bytes.split(|b| *b == b'\n').any(|line| {
        line.first().is_some_and(|first| {
            matches!(first, b'<' | b'|' | b'=' | b'>')
                && line.iter().take_while(|b| *b == first).count() >= width
        })
    })
}

fn parse_stages(bytes: &[u8]) -> Result<Vec<ConflictEntry>> {
    let mut entries: BTreeMap<PathBuf, ConflictEntry> = BTreeMap::new();
    for record in bytes.split(|b| *b == 0).filter(|v| !v.is_empty()) {
        let tab = record
            .iter()
            .position(|b| *b == b'\t')
            .ok_or_else(|| refusal("stage record has no path separator"))?;
        let fields: Vec<_> = record[..tab].split(|b| *b == b' ').collect();
        if fields.len() != 3 {
            return Err(refusal("invalid index stage record"));
        }
        let mode = std::str::from_utf8(fields[0])
            .map_err(|_| refusal("invalid mode"))?
            .to_owned();
        let oid = std::str::from_utf8(fields[1])
            .map_err(|_| refusal("invalid object ID"))?
            .to_owned();
        if !matches!(mode.as_str(), "100644" | "100755" | "120000" | "160000")
            || !valid_object_id(&oid)
        {
            return Err(refusal("invalid stage mode or object ID"));
        }
        let stage = match fields[2] {
            b"0" => continue,
            b"1" => 1,
            b"2" => 2,
            b"3" => 3,
            _ => return Err(refusal("invalid stage number")),
        };
        let path = status::path_from_bytes(&record[tab + 1..]).map_err(refusal)?;
        if path.as_os_str().is_empty()
            || path
                .components()
                .any(|c| !matches!(c, Component::Normal(_)))
        {
            return Err(refusal("invalid conflict path"));
        }
        let entry = entries.entry(path.clone()).or_insert(ConflictEntry {
            path,
            marker_width: 7,
            base: None,
            current: None,
            other: None,
        });
        let slot = match stage {
            1 => &mut entry.base,
            2 => &mut entry.current,
            _ => &mut entry.other,
        };
        if slot.replace(ConflictStage { mode, oid, stage }).is_some() {
            return Err(refusal("duplicate index stage"));
        }
    }
    Ok(entries.into_values().collect())
}

fn parse_preview(bytes: &[u8], clean: bool) -> Result<MergePreviewOutcome> {
    let mut tokens = bytes.split(|b| *b == 0);
    let tree_oid = std::str::from_utf8(tokens.next().unwrap_or_default())
        .map_err(|_| refusal("invalid preview tree"))?
        .to_owned();
    if !valid_object_id(&tree_oid) {
        return Err(refusal(
            "Git lacks the required machine-delimited merge-tree output",
        ));
    }
    let mut stages = Vec::new();
    for token in tokens.by_ref() {
        if token.is_empty() {
            break;
        }
        stages.extend_from_slice(token);
        stages.push(0);
    }
    let conflicts = parse_stages(&stages)?;
    let mut messages = Vec::new();
    while let Some(count) = tokens.next() {
        if count.is_empty() {
            continue;
        }
        let count = std::str::from_utf8(count)
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .filter(|v| *v <= 4096)
            .ok_or_else(|| refusal("invalid conflict path count"))?;
        let mut paths = Vec::new();
        for _ in 0..count {
            paths.push(
                status::path_from_bytes(
                    tokens
                        .next()
                        .ok_or_else(|| refusal("missing conflict path"))?,
                )
                .map_err(refusal)?,
            );
        }
        let kind = String::from_utf8_lossy(
            tokens
                .next()
                .ok_or_else(|| refusal("missing conflict kind"))?,
        )
        .into_owned();
        let message = String::from_utf8_lossy(
            tokens
                .next()
                .ok_or_else(|| refusal("missing conflict message"))?,
        )
        .into_owned();
        messages.push(MergeConflictMessage {
            paths,
            kind,
            message,
        });
    }
    if clean && !conflicts.is_empty() {
        return Err(refusal("clean merge reported unmerged stages"));
    }
    Ok(MergePreviewOutcome::Merge {
        has_conflicts: !clean,
        tree_oid,
        conflicts,
        messages,
    })
}
