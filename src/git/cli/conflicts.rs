// SPDX-License-Identifier: MPL-2.0

use super::merge::{refusal, stale};
use super::*;
use crate::git::BufferRevisionGuard;
use crate::git::merge::*;
use std::collections::BTreeMap;

impl GitCliProvider {
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
                format!("Current: {}", self.merge_status(repo)?.head.label()),
                format!("Other: {}", parents.join(", ")),
            ),
            RepositoryOperation::Rebase => (
                "Onto checkout (stage 2)".into(),
                "Replayed commit (stage 3)".into(),
            ),
            _ => ("Stage 2".into(), "Stage 3".into()),
        };
        let head_oid = if matches!(self.merge_status(repo)?.head, Head::Unborn(_)) {
            None
        } else {
            self.head_oid(repo)?
        };
        Ok(ConflictInventory {
            current_identity,
            other_identity,
            operation: self.inspect_operation(repo)?,
            head_oid,
            entries,
            index_identity: crate::hash::sha256_hex(&index),
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
            && (inventory.related_paths(&entry.path).len() > 1
                || entry
                    .current
                    .as_ref()
                    .is_some_and(|s| !matches!(s.mode.as_str(), "100644" | "100755" | "120000"))
                || entry
                    .other
                    .as_ref()
                    .is_some_and(|s| !matches!(s.mode.as_str(), "100644" | "100755" | "120000")))
        {
            return Err(refusal(
                "this structural conflict requires manual file management; review and stage each affected path",
            ));
        }
        let disk_identity = self.merge_disk(repo)?;
        let reviewed_content = match choice {
            ResolutionChoice::SavedFile {
                allow_literal_markers,
            } => match std::fs::symlink_metadata(path) {
                Err(e) if e.kind() == io::ErrorKind::NotFound => BaseContent::Absent,
                Err(e) => return Err(refusal(e.to_string())),
                Ok(metadata) if metadata.is_file() => {
                    let bytes = read_file_for_comparison(path, self.max_output_bytes)?;
                    if !allow_literal_markers && marker_like(&bytes, entry.marker_width) {
                        return Err(refusal(
                            "file contains conflict markers; resolve them or explicitly review literal marker text",
                        ));
                    }
                    if crate::external_open::is_binary(&bytes, true) {
                        BaseContent::Binary
                    } else {
                        String::from_utf8(bytes)
                            .map(BaseContent::Text)
                            .unwrap_or(BaseContent::Binary)
                    }
                }
                Ok(metadata) if metadata.file_type().is_symlink() => {
                    let target = std::fs::read_link(path).map_err(|e| refusal(e.to_string()))?;
                    if target.as_os_str().as_encoded_bytes().len() > self.max_output_bytes {
                        return Err(GitError::TooLarge {
                            command: format!("read link {}", path.display()),
                            limit: self.max_output_bytes,
                        });
                    }
                    target
                        .to_str()
                        .map(|target| BaseContent::Text(target.into()))
                        .unwrap_or(BaseContent::Binary)
                }
                Ok(_) => {
                    return Err(refusal(
                        "structural path requires explicit manual file management",
                    ));
                }
            },
            ResolutionChoice::Current | ResolutionChoice::Other => {
                let selected = if choice == ResolutionChoice::Current {
                    &entry.current
                } else {
                    &entry.other
                };
                match selected {
                    Some(stage) => self.object_content(repo, &stage.oid)?,
                    None => BaseContent::Absent,
                }
            }
        };
        // The displayed content must belong to the captured mutation authority.
        if self.merge_disk(repo)? != disk_identity
            || self.read_conflicts(repo)? != inventory
            || !guard.is_valid()
        {
            return Err(stale());
        }
        let mut plan = ResolutionPlan {
            review_identity: String::new(),
            repository: repo.clone(),
            repository_identity: self.merge_repository_identity(repo)?,
            path: path.to_owned(),
            choice,
            entry,
            reviewed_content,
            inventory,
            disk_identity,
            guard,
        };
        plan.review_identity = plan.current_review_identity();
        Ok(plan)
    }
    pub(super) fn apply_resolution(
        &self,
        repo: &Repository,
        plan: &ResolutionPlan,
    ) -> Result<ConflictInventory> {
        if plan.review_identity != plan.current_review_identity()
            || repo != &plan.repository
            || self.merge_repository_identity(repo)? != plan.repository_identity
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
                if let Some(selected) = selected {
                    let side = if plan.choice == ResolutionChoice::Current {
                        "--ours"
                    } else {
                        "--theirs"
                    };
                    self.run(
                        repo.workdir(),
                        &[
                            OsStr::new("--literal-pathspecs"),
                            OsStr::new("checkout"),
                            OsStr::new(side),
                            OsStr::new("--"),
                            relative.as_os_str(),
                        ],
                    )?;
                    // Checkout must succeed before replacing the unmerged stages.
                    // Adding the worktree file would infer its mode from disk,
                    // losing reviewed executable/symlink modes on some platforms.
                    self.run(
                        repo.workdir(),
                        &[
                            OsStr::new("update-index"),
                            OsStr::new("--add"),
                            OsStr::new("--cacheinfo"),
                            OsStr::new(&selected.mode),
                            OsStr::new(&selected.oid),
                            relative.as_os_str(),
                        ],
                    )?;
                } else {
                    // Git owns path validation and refuses an unexpected directory.
                    self.run(
                        repo.workdir(),
                        &[
                            OsStr::new("--literal-pathspecs"),
                            OsStr::new("rm"),
                            OsStr::new("-f"),
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
        let mut plan = MergeCompletionPlan {
            review_identity: String::new(),
            repository: repo.clone(),
            repository_identity: self.merge_repository_identity(repo)?,
            inventory,
            message,
            staged_diff,
            disk_identity: self.merge_disk(repo)?,
            markers_identity: self.completion_markers(repo)?,
            settings_identity: crate::hash::sha256_hex(
                &self.merge_read(repo, &["config", "--null", "--list"])?,
            ),
            guard,
        };
        plan.review_identity = plan.current_review_identity();
        Ok(plan)
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
        if plan.review_identity != plan.current_review_identity()
            || repo != &plan.repository
            || self.merge_repository_identity(repo)? != plan.repository_identity
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

pub(super) fn parse_stages(bytes: &[u8]) -> Result<Vec<ConflictEntry>> {
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
