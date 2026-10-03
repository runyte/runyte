// SPDX-License-Identifier: MPL-2.0

use super::conflicts::parse_stages;
use super::*;
use crate::git::merge::*;
use crate::git::{BufferRevisionGuard, ComparisonTarget};
use std::collections::BTreeSet;
use std::sync::{Mutex, OnceLock};

pub(super) fn refusal(detail: impl Into<String>) -> GitError {
    GitError::Malformed {
        command: "reviewed merge".into(),
        detail: detail.into(),
    }
}
pub(super) fn stale() -> GitError {
    refusal("repository, refs, index, disk, settings or buffer changed; review again")
}

impl GitCliProvider {
    pub(super) fn merge_repository_identity(&self, repo: &Repository) -> Result<String> {
        let mut identity = String::new();
        for path in [repo.workdir(), repo.git_dir(), repo.common_dir()] {
            let canonical = path.canonicalize().map_err(|e| refusal(e.to_string()))?;
            let metadata = std::fs::metadata(path).map_err(|e| refusal(e.to_string()))?;
            identity.push_str(&format!("{canonical:?}"));
            #[cfg(unix)]
            {
                use std::os::unix::fs::MetadataExt;
                identity.push_str(&format!(":{}:{}", metadata.dev(), metadata.ino()));
            }
            #[cfg(not(unix))]
            {
                identity.push_str(&format!(":{:?}", metadata.created()));
            }
        }
        Ok(crate::hash::sha256_hex(identity.as_bytes()))
    }
    pub(super) fn merge_status(&self, repo: &Repository) -> Result<RepositoryStatus> {
        let output = self.merge_read(
            repo,
            &[
                "status",
                "--porcelain=v2",
                "--branch",
                "-z",
                "--untracked-files=all",
                "--ignore-submodules=none",
            ],
        )?;
        status::parse(&output).map_err(refusal)
    }
    fn merge_fingerprint(&self, repo: &Repository) -> Result<RepositoryFingerprint> {
        Ok(RepositoryFingerprint {
            head: self.head_oid(repo)?,
            index: crate::hash::sha256_hex(&self.merge_read(repo, &["ls-files", "--stage", "-z"])?),
        })
    }
    pub(super) fn merge_read(&self, repo: &Repository, args: &[&str]) -> Result<Vec<u8>> {
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
    fn merge_settings(&self, repo: &Repository) -> Result<String> {
        let config = self.merge_read(repo, &["config", "--null", "--list"])?;
        for item in config.split(|b| *b == 0).filter(|v| !v.is_empty()) {
            let mut fields = item.splitn(2, |b| *b == b'\n');
            let key = String::from_utf8_lossy(fields.next().unwrap_or_default()).to_lowercase();
            let value = String::from_utf8_lossy(fields.next().unwrap_or_default()).to_lowercase();
            if key.ends_with(".mergeoptions")
                || matches!(key.as_str(), "merge.renormalize" | "core.sparsecheckout")
                    && !matches!(value.as_str(), "false" | "no" | "off" | "0")
            {
                return Err(refusal(format!(
                    "reviewed merge does not support configuration {key}; use Git directly"
                )));
            }
        }
        Ok(crate::hash::sha256_hex(&config))
    }
    pub(super) fn merge_disk(&self, repo: &Repository) -> Result<String> {
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

                    let mut file = crate::path_safety::open_regular_file(&absolute, false)
                        .map_err(|e| refusal(e.to_string()))?;
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
    fn reject_tree_attributes(&self, repo: &Repository, oid: &str) -> Result<String> {
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
        if !fields.len().is_multiple_of(3) {
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
        Ok(crate::hash::sha256_hex(&attributes))
    }
    fn effective_attributes(
        &self,
        repo: &Repository,
        destination: &str,
        source: &str,
    ) -> Result<String> {
        let unknown = self.merge_read(
            repo,
            &[
                "ls-files",
                "--others",
                "-z",
                "--",
                ".gitattributes",
                ":(glob)**/.gitattributes",
            ],
        )?;
        if !unknown.is_empty() {
            return Err(refusal(
                "untracked or ignored .gitattributes can change application; review them with Git directly",
            ));
        }
        let bases = self.merge_read(repo, &["merge-base", "--all", destination, source]).map_err(|error| match error { GitError::Failed { code: Some(1), .. } => refusal("selected branches have no common ancestor; reviewed merging refuses unrelated histories"), error => error })?;
        let mut hashes = self.reject_tree_attributes(repo, destination)?;
        hashes.push_str(&self.reject_tree_attributes(repo, source)?);
        for base in String::from_utf8_lossy(&bases).lines() {
            hashes.push_str(&self.reject_tree_attributes(repo, base)?);
        }
        Ok(crate::hash::sha256_hex(hashes.as_bytes()))
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
        let (clean, bytes) = self.run_bounded_until(repo.workdir(), &["merge-tree", "--write-tree", "-z", "--messages", oid, oid], self.max_output_bytes, self.local_read_timeout, true).map_err(|e| match e {
            GitError::Failed { .. } => refusal("Git lacks the required merge-tree --write-tree -z --messages capability; upgrade Git"),
            other => other,
        })?;
        if !clean {
            return Err(refusal(
                "Git merge-tree capability probe unexpectedly reported a conflict; upgrade Git",
            ));
        }
        let _ = parse_preview(&bytes, clean)?;
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
        let status = self.merge_status(repo)?;
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
        let attributes_identity = self.effective_attributes(repo, &destination_oid, &source_oid)?;
        let flags = self.merge_read(repo, &["ls-files", "-v", "-z"])?;
        if flags.split(|b| *b == 0).any(|line| {
            line.first()
                .is_some_and(|flag| flag.is_ascii_lowercase() || *flag == b'S')
        }) {
            return Err(refusal(
                "reviewed merging does not support assume-unchanged or sparse/skip-worktree files",
            ));
        }
        let fingerprint = self.merge_fingerprint(repo)?;
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
        let mut plan = MergePlan {
            review_identity: String::new(),
            comparison,
            repository: repo.clone(),
            repository_identity: self.merge_repository_identity(repo)?,
            destination_reference,
            destination_oid,
            source_reference: source.into(),
            source_oid,
            outcome,
            fingerprint,
            disk_identity,
            settings_identity,
            attributes_identity,
            guard,
        };
        plan.review_identity = plan.current_review_identity();
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
        if plan.review_identity != plan.current_review_identity()
            || repo != &plan.repository
            || self.merge_repository_identity(repo)? != plan.repository_identity
            || !plan.guard.is_valid()
            || self.inspect_operation(repo)? != RepositoryOperation::Idle
            || self.merge_fingerprint(repo)? != plan.fingerprint
            || self.merge_disk(repo)? != plan.disk_identity
            || self.merge_settings(repo)? != plan.settings_identity
            || self.effective_attributes(repo, &plan.destination_oid, &plan.source_oid)?
                != plan.attributes_identity
            || self.merge_tip(repo, &plan.source_reference)? != plan.source_oid
            || self.merge_tip(repo, &plan.destination_reference)? != plan.destination_oid
        {
            return Err(stale());
        }
        if !self.merge_status(repo)?.files.is_empty() {
            return Err(stale());
        }
        let branch = plan
            .destination_reference
            .strip_prefix("refs/heads/")
            .unwrap_or_default();
        if self.merge_status(repo)?.head != Head::Branch(branch.into()) {
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
                "-c",
                "submodule.recurse=false",
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

#[cfg(test)]
#[path = "../tests/merge_parsing.rs"]
mod tests;
