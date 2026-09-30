// SPDX-License-Identifier: MPL-2.0

use runyte::git::{
    BaseContent, BufferRevisionGuard, GitCliProvider, GitError, GitProvider, MergeApplied,
    MergePreviewOutcome, Repository, RepositoryOperation, ResolutionChoice,
    conflict_regions::parse_conflict_regions,
};
use std::{
    fs,
    path::PathBuf,
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
};

static NEXT: AtomicU64 = AtomicU64::new(1);
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        Self::with_format("sha1")
    }
    fn with_format(format: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "runyte-merge-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).unwrap();
        let f = Self(path.canonicalize().unwrap());
        f.git(&[
            "init",
            "-q",
            "--initial-branch=main",
            &format!("--object-format={format}"),
        ]);
        for (key, value) in [
            ("user.name", "Runyte Test"),
            ("user.email", "runyte@example.invalid"),
            ("commit.gpgsign", "false"),
            ("core.autocrlf", "false"),
            ("core.hooksPath", ".git/hooks"),
        ] {
            f.git(&["config", key, value]);
        }
        f.write("file", "base\n");
        f.commit("base");
        f
    }
    fn repo(&self) -> Repository {
        GitCliProvider::new("git")
            .discover(&self.0)
            .unwrap()
            .unwrap()
    }
    fn command(&self, args: &[&str]) -> std::process::Output {
        Command::new("git")
            .args(args)
            .current_dir(&self.0)
            .env("XDG_CONFIG_HOME", self.0.join("config-home"))
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .output()
            .unwrap()
    }
    fn git(&self, args: &[&str]) -> String {
        let out = self.command(args);
        assert!(
            out.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).unwrap().trim().to_owned()
    }
    fn write(&self, path: &str, text: &str) {
        fs::write(self.0.join(path), text).unwrap();
    }
    fn commit(&self, message: &str) {
        self.git(&["add", "-A"]);
        self.git(&["commit", "-qm", message]);
    }
    fn divergent(&self, conflict: bool) {
        self.git(&["checkout", "-qb", "feature"]);
        self.write(if conflict { "file" } else { "other" }, "other\n");
        self.commit("feature");
        self.git(&["checkout", "-q", "main"]);
        self.write("file", "current\n");
        self.commit("current");
    }
    fn plan(&self) -> runyte::git::MergePlan {
        GitCliProvider::new("git")
            .prepare_merge(
                &self.repo(),
                "refs/heads/feature",
                BufferRevisionGuard::new(),
            )
            .unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn preview_preserves_index_and_disk_and_clean_application_matches_tree() {
    let f = Fixture::new();
    f.divergent(false);
    let p = GitCliProvider::new("git");
    let index = fs::read(f.0.join(".git/index")).unwrap();
    let disk = fs::read(f.0.join("file")).unwrap();
    let head = f.git(&["rev-parse", "HEAD"]);
    let plan = f.plan();
    assert_eq!(fs::read(f.0.join(".git/index")).unwrap(), index);
    assert_eq!(fs::read(f.0.join("file")).unwrap(), disk);
    assert_eq!(f.git(&["rev-parse", "HEAD"]), head);
    let MergePreviewOutcome::Merge {
        tree_oid,
        has_conflicts,
        ..
    } = &plan.outcome
    else {
        panic!()
    };
    assert!(!has_conflicts);
    assert_eq!(plan.comparison.files.len(), 1);
    assert_eq!(
        p.apply_merge(&f.repo(), &plan).unwrap().outcome,
        MergeApplied::PendingCommit
    );
    assert_eq!(f.git(&["write-tree"]), *tree_oid);
    assert_eq!(f.git(&["rev-parse", "HEAD"]), head);
    let completion = p
        .prepare_merge_completion(&f.repo(), BufferRevisionGuard::new())
        .unwrap();
    p.commit_merge(&f.repo(), &completion, "reviewed merge")
        .unwrap();
    assert_eq!(
        f.git(&["rev-list", "--parents", "-n", "1", "HEAD"])
            .split_whitespace()
            .count(),
        3
    );
    assert_eq!(
        p.operation_state(&f.repo()).unwrap(),
        RepositoryOperation::Idle
    );
}

#[test]
fn fast_forward_and_contained_are_explicit_and_do_not_create_merge_commit() {
    let f = Fixture::new();
    f.git(&["checkout", "-qb", "feature"]);
    f.write("other", "other\n");
    f.commit("feature");
    let tip = f.git(&["rev-parse", "HEAD"]);
    f.git(&["checkout", "-q", "main"]);
    let p = GitCliProvider::new("git");
    let plan = f.plan();
    assert_eq!(plan.outcome, MergePreviewOutcome::FastForward);
    assert_eq!(
        p.apply_merge(&f.repo(), &plan).unwrap().outcome,
        MergeApplied::FastForward
    );
    assert_eq!(f.git(&["rev-parse", "HEAD"]), tip);
    let plan = f.plan();
    assert_eq!(plan.outcome, MergePreviewOutcome::AlreadyContained);
    assert!(plan.comparison.files.is_empty());
    assert_eq!(
        p.apply_merge(&f.repo(), &plan).unwrap().outcome,
        MergeApplied::AlreadyContained
    );
    assert!(
        p.prepare_merge_completion(&f.repo(), BufferRevisionGuard::new())
            .is_err()
    );
}

#[test]
fn actual_conflicts_and_stage_blobs_are_authoritative() {
    let f = Fixture::new();
    f.divergent(true);
    let p = GitCliProvider::new("git");
    let plan = f.plan();
    let MergePreviewOutcome::Merge {
        has_conflicts,
        conflicts,
        ..
    } = &plan.outcome
    else {
        panic!()
    };
    assert!(*has_conflicts);
    assert_eq!(conflicts.len(), 1);
    let applied = p.apply_merge(&f.repo(), &plan).unwrap();
    assert_eq!(applied.outcome, MergeApplied::Conflicted);
    assert!(applied.diagnostic.is_some());
    let entry = &applied.inventory.entries[0];
    assert_eq!(entry.kind(), "content");
    assert_eq!(entry.marker_width, 7);
    let sides = p.conflict_sides(&f.repo(), entry).unwrap();
    assert_eq!(sides[0], runyte::git::BaseContent::Text("base\n".into()));
    assert_eq!(sides[1], runyte::git::BaseContent::Text("current\n".into()));
    assert_eq!(sides[2], runyte::git::BaseContent::Text("other\n".into()));
    assert!(
        p.prepare_resolution(
            &f.repo(),
            &f.0.join("file"),
            ResolutionChoice::SavedFile {
                allow_literal_markers: false
            },
            BufferRevisionGuard::new()
        )
        .is_err()
    );
    let completion = p
        .prepare_merge_completion(&f.repo(), BufferRevisionGuard::new())
        .unwrap();
    assert!(
        p.commit_merge(&f.repo(), &completion, "not resolved")
            .is_err()
    );
    f.write("file", "resolved manually\n");
    let resolution = p
        .prepare_resolution(
            &f.repo(),
            &f.0.join("file"),
            ResolutionChoice::SavedFile {
                allow_literal_markers: false,
            },
            BufferRevisionGuard::new(),
        )
        .unwrap();
    assert!(
        p.resolve_conflict(&f.repo(), &resolution)
            .unwrap()
            .entries
            .is_empty()
    );
    assert_eq!(
        fs::read_to_string(f.0.join("file")).unwrap(),
        "resolved manually\n"
    );
}

#[test]
fn stale_merge_refs_index_disk_settings_and_editor_guard_refuse_mutation() {
    for changed in [
        "source",
        "destination",
        "index",
        "disk",
        "settings",
        "guard",
        "branch",
    ] {
        let f = Fixture::new();
        f.divergent(false);
        let p = GitCliProvider::new("git");
        let plan = f.plan();
        let before = f.git(&["rev-parse", "HEAD"]);
        match changed {
            "source" => {
                f.git(&["update-ref", "refs/heads/feature", &before]);
            }
            "destination" => {
                let source = f.git(&["rev-parse", "feature"]);
                f.git(&["update-ref", "refs/heads/main", &source]);
            }
            "index" => {
                f.write("file", "index change\n");
                f.git(&["add", "file"]);
            }
            "disk" => f.write("file", "same length new\n"),
            "settings" => {
                f.git(&["config", "merge.conflictstyle", "diff3"]);
            }
            "guard" => plan.invalidate(),
            "branch" => {
                f.git(&["checkout", "-qb", "switched"]);
            }
            _ => unreachable!(),
        }
        let head = f.git(&["rev-parse", "HEAD"]);
        assert!(p.apply_merge(&f.repo(), &plan).is_err(), "{changed}");
        assert_eq!(f.git(&["rev-parse", "HEAD"]), head);
        assert!(!f.0.join(".git/MERGE_HEAD").exists());
    }
}

#[test]
fn resolution_review_pins_disk_and_index_and_can_explicitly_accept_literal_markers() {
    let f = Fixture::new();
    f.divergent(true);
    let p = GitCliProvider::new("git");
    p.apply_merge(&f.repo(), &f.plan()).unwrap();
    let path = f.0.join("file");
    let review = p
        .prepare_resolution(
            &f.repo(),
            &path,
            ResolutionChoice::SavedFile {
                allow_literal_markers: true,
            },
            BufferRevisionGuard::new(),
        )
        .unwrap();
    f.write("file", "new edit\n");
    assert!(p.resolve_conflict(&f.repo(), &review).is_err());
    assert_eq!(p.conflicts(&f.repo()).unwrap().entries.len(), 1);
    f.write("file", "<<<<<<< intentional\n");
    let review = p
        .prepare_resolution(
            &f.repo(),
            &path,
            ResolutionChoice::SavedFile {
                allow_literal_markers: true,
            },
            BufferRevisionGuard::new(),
        )
        .unwrap();
    assert_eq!(
        review.reviewed_content,
        BaseContent::Text("<<<<<<< intentional\n".into())
    );
    assert!(
        p.resolve_conflict(&f.repo(), &review)
            .unwrap()
            .entries
            .is_empty()
    );
}

#[test]
fn whole_regular_side_and_modify_delete_choices_stage_only_reviewed_path() {
    let f = Fixture::new();
    f.divergent(true);
    let p = GitCliProvider::new("git");
    p.apply_merge(&f.repo(), &f.plan()).unwrap();
    let review = p
        .prepare_resolution(
            &f.repo(),
            &f.0.join("file"),
            ResolutionChoice::Other,
            BufferRevisionGuard::new(),
        )
        .unwrap();
    assert_eq!(review.reviewed_content, BaseContent::Text("other\n".into()));
    p.resolve_conflict(&f.repo(), &review).unwrap();
    assert_eq!(fs::read_to_string(f.0.join("file")).unwrap(), "other\n");
    let f = Fixture::new();
    f.git(&["checkout", "-qb", "feature"]);
    f.git(&["rm", "file"]);
    f.commit("delete");
    f.git(&["checkout", "-q", "main"]);
    f.write("file", "current\n");
    f.commit("modify");
    p.apply_merge(&f.repo(), &f.plan()).unwrap();
    let inventory = p.conflicts(&f.repo()).unwrap();
    assert!(inventory.entries[0].other.is_none());
    let review = p
        .prepare_resolution(
            &f.repo(),
            &f.0.join("file"),
            ResolutionChoice::Other,
            BufferRevisionGuard::new(),
        )
        .unwrap();
    assert_eq!(review.reviewed_content, BaseContent::Absent);
    p.resolve_conflict(&f.repo(), &review).unwrap();
    assert!(!f.0.join("file").exists());
}

#[test]
fn saved_resolution_preview_classifies_exact_disk_content_and_is_sealed() {
    let f = Fixture::new();
    f.divergent(true);
    let p = GitCliProvider::new("git");
    p.apply_merge(&f.repo(), &f.plan()).unwrap();
    let path = f.0.join("file");
    for (bytes, expected) in [
        (
            b"saved result\n".as_slice(),
            BaseContent::Text("saved result\n".into()),
        ),
        (b"binary\0result".as_slice(), BaseContent::Binary),
    ] {
        fs::write(&path, bytes).unwrap();
        let mut review = p
            .prepare_resolution(
                &f.repo(),
                &path,
                ResolutionChoice::SavedFile {
                    allow_literal_markers: false,
                },
                BufferRevisionGuard::new(),
            )
            .unwrap();
        assert_eq!(review.reviewed_content, expected);
        review.reviewed_content = BaseContent::Text("substituted preview".into());
        assert!(p.resolve_conflict(&f.repo(), &review).is_err());
        assert_eq!(p.conflicts(&f.repo()).unwrap().entries.len(), 1);
    }
    fs::remove_file(&path).unwrap();
    let review = p
        .prepare_resolution(
            &f.repo(),
            &path,
            ResolutionChoice::SavedFile {
                allow_literal_markers: false,
            },
            BufferRevisionGuard::new(),
        )
        .unwrap();
    assert_eq!(review.reviewed_content, BaseContent::Absent);
    assert!(
        p.resolve_conflict(&f.repo(), &review)
            .unwrap()
            .entries
            .is_empty()
    );
    assert!(f.git(&["ls-files", "--stage", "file"]).is_empty());
}

#[test]
fn completion_guards_orig_head_disk_merge_identity_and_allows_unchanged_tree_merge() {
    for changed in ["orig_head", "disk", "merge_head", "guard"] {
        let f = Fixture::new();
        f.divergent(false);
        let p = GitCliProvider::new("git");
        p.apply_merge(&f.repo(), &f.plan()).unwrap();
        let completion = p
            .prepare_merge_completion(&f.repo(), BufferRevisionGuard::new())
            .unwrap();
        match changed {
            "orig_head" => {
                f.git(&["update-ref", "ORIG_HEAD", &f.git(&["rev-parse", "feature"])]);
            }
            "disk" => f.write("file", "post-review edit\n"),
            "merge_head" => {
                let oid = f.git(&["rev-parse", "HEAD"]);
                fs::write(f.0.join(".git/MERGE_HEAD"), format!("{oid}\n")).unwrap();
            }
            "guard" => completion.invalidate(),
            _ => unreachable!(),
        }
        let head = f.git(&["rev-parse", "HEAD"]);
        assert!(p.abort_merge(&f.repo(), &completion).is_err(), "{changed}");
        assert_eq!(f.git(&["rev-parse", "HEAD"]), head);
    }
    let f = Fixture::new();
    f.git(&["checkout", "-qb", "feature"]);
    f.git(&["commit", "--allow-empty", "-qm", "empty feature"]);
    f.git(&["checkout", "-q", "main"]);
    f.git(&["commit", "--allow-empty", "-qm", "empty main"]);
    let p = GitCliProvider::new("git");
    let plan = f.plan();
    assert!(plan.comparison.files.is_empty());
    p.apply_merge(&f.repo(), &plan).unwrap();
    let completion = p
        .prepare_merge_completion(&f.repo(), BufferRevisionGuard::new())
        .unwrap();
    assert!(completion.staged_diff.is_empty());
    p.commit_merge(&f.repo(), &completion, "empty merge")
        .unwrap();
    assert_eq!(
        f.git(&["rev-list", "--parents", "-n", "1", "HEAD"])
            .split_whitespace()
            .count(),
        3
    );
}

#[test]
fn abort_uses_git_flow_for_clean_and_conflicted_pending_merges() {
    for conflict in [false, true] {
        let f = Fixture::new();
        f.divergent(conflict);
        let p = GitCliProvider::new("git");
        let head = f.git(&["rev-parse", "HEAD"]);
        p.apply_merge(&f.repo(), &f.plan()).unwrap();
        let review = p
            .prepare_merge_completion(&f.repo(), BufferRevisionGuard::new())
            .unwrap();
        p.abort_merge(&f.repo(), &review).unwrap();
        assert_eq!(f.git(&["rev-parse", "HEAD"]), head);
        assert_eq!(
            p.operation_state(&f.repo()).unwrap(),
            RepositoryOperation::Idle
        );
    }
}

#[test]
fn normal_attributes_and_rename_preview_match_actual_tree_custom_driver_is_refused() {
    let f = Fixture::new();
    f.write(".gitattributes", "* text eol=lf\n");
    f.commit("attributes");
    f.git(&["checkout", "-qb", "feature"]);
    f.git(&["mv", "file", "renamed"]);
    f.commit("rename");
    f.git(&["checkout", "-q", "main"]);
    f.write("other", "current\n");
    f.commit("current");
    let p = GitCliProvider::new("git");
    let plan = f.plan();
    let MergePreviewOutcome::Merge { tree_oid, .. } = &plan.outcome else {
        panic!()
    };
    p.apply_merge(&f.repo(), &plan).unwrap();
    assert_eq!(f.git(&["write-tree"]), *tree_oid);
    let f = Fixture::new();
    f.write(".gitattributes", "file merge=custom\n");
    f.commit("custom attribute");
    f.divergent(true);
    f.git(&["config", "merge.custom.driver", "false"]);
    assert!(
        p.prepare_merge(&f.repo(), "refs/heads/feature", BufferRevisionGuard::new())
            .is_err()
    );
    assert!(!f.0.join(".git/MERGE_HEAD").exists());
}

#[test]
fn configured_short_markers_are_detected_for_staging_and_region_parser() {
    let f = Fixture::new();
    f.write(".gitattributes", "file conflict-marker-size=3\n");
    f.commit("markers");
    f.divergent(true);
    let p = GitCliProvider::new("git");
    p.apply_merge(&f.repo(), &f.plan()).unwrap();
    let inventory = p.conflicts(&f.repo()).unwrap();
    assert_eq!(inventory.entries[0].marker_width, 3);
    assert!(
        p.prepare_resolution(
            &f.repo(),
            &f.0.join("file"),
            ResolutionChoice::SavedFile {
                allow_literal_markers: false
            },
            BufferRevisionGuard::new()
        )
        .is_err()
    );
    let text = fs::read_to_string(f.0.join("file")).unwrap();
    assert_eq!(parse_conflict_regions(&text, 3).unwrap().len(), 1);
}

#[test]
fn non_merge_operation_state_is_worktree_local_and_disables_merge_completion() {
    let f = Fixture::new();
    let p = GitCliProvider::new("git");
    for (name, state) in [
        ("rebase-merge", RepositoryOperation::Rebase),
        ("CHERRY_PICK_HEAD", RepositoryOperation::CherryPick),
        ("REVERT_HEAD", RepositoryOperation::Revert),
        ("BISECT_LOG", RepositoryOperation::Bisect),
        ("sequencer", RepositoryOperation::Other),
    ] {
        fs::write(f.0.join(".git").join(name), "state").unwrap();
        assert_eq!(p.operation_state(&f.repo()).unwrap(), state);
        assert!(
            p.prepare_merge_completion(&f.repo(), BufferRevisionGuard::new())
                .is_err()
        );
        fs::remove_file(f.0.join(".git").join(name)).unwrap();
    }
}

#[test]
fn unsupported_refs_dirty_state_and_output_limits_refuse_preview() {
    let f = Fixture::new();
    f.divergent(false);
    let p = GitCliProvider::new("git");
    assert!(
        p.prepare_merge(&f.repo(), "--help", BufferRevisionGuard::new())
            .is_err()
    );
    f.write("untracked", "new\n");
    assert!(matches!(
        p.prepare_merge(&f.repo(), "refs/heads/feature", BufferRevisionGuard::new()),
        Err(GitError::DirtyWorktree { .. })
    ));
    fs::remove_file(f.0.join("untracked")).unwrap();
    assert!(matches!(
        GitCliProvider::new("git")
            .with_max_output_bytes(1)
            .prepare_merge(&f.repo(), "refs/heads/feature", BufferRevisionGuard::new()),
        Err(GitError::TooLarge { .. })
    ));
}

#[test]
fn pure_regions_preserve_unicode_and_diff3_base_and_refuse_ambiguity() {
    for base in ["", "||||||| Base\nbase\n"] {
        let text =
            format!("before 🦀\n<<<<<<< Current\nα\n{base}=======\nβ\n>>>>>>> Other\nafter\n");
        let region = parse_conflict_regions(&text, 7).unwrap().remove(0);
        let chars: Vec<_> = text.chars().collect();
        assert_eq!(
            chars[region.current.clone()].iter().collect::<String>(),
            "α\n"
        );
        assert_eq!(
            chars[region.other.clone()].iter().collect::<String>(),
            "β\n"
        );
        assert_eq!(region.base.is_some(), !base.is_empty());
        assert!(region.range.start > 0);
        assert!(region.range.end < chars.len());
    }
    for text in [
        "<<<<<<< a\nx\n",
        "=======\n",
        "<<<<<<< a\n<<<<<<< b\n=======\n>>>>>>> c\n",
    ] {
        assert!(parse_conflict_regions(text, 7).is_err());
    }
    assert!(parse_conflict_regions("", 0).is_err());
}

#[cfg(unix)]
#[test]
fn failed_commit_hook_keeps_merge_and_message_available() {
    let f = Fixture::new();
    f.divergent(false);
    let p = GitCliProvider::new("git");
    p.apply_merge(&f.repo(), &f.plan()).unwrap();
    let hook = f.0.join(".git/hooks/pre-commit");
    std::os::unix::fs::symlink(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/fixtures/stand-in"),
        &hook,
    )
    .unwrap();
    fs::write(hook.with_extension("behavior"), "exit 1\n").unwrap();
    let completion = p
        .prepare_merge_completion(&f.repo(), BufferRevisionGuard::new())
        .unwrap();
    assert!(
        p.commit_merge(&f.repo(), &completion, "retry message")
            .is_err()
    );
    assert!(matches!(
        p.operation_state(&f.repo()).unwrap(),
        RepositoryOperation::Merge { .. }
    ));
    fs::remove_file(hook).unwrap();
    p.commit_merge(&f.repo(), &completion, "retry message")
        .unwrap();
}

#[test]
fn cached_rerere_resolution_cannot_automatically_resolve_or_stage_reviewed_merge() {
    let f = Fixture::new();
    f.divergent(true);
    f.git(&["config", "rerere.enabled", "true"]);
    f.git(&["config", "rerere.autoupdate", "true"]);
    assert!(
        !f.command(&["merge", "--no-commit", "feature"])
            .status
            .success()
    );
    f.write("file", "cached resolution\n");
    f.git(&["rerere"]);
    f.git(&["merge", "--abort"]);
    let p = GitCliProvider::new("git");
    let applied = p.apply_merge(&f.repo(), &f.plan()).unwrap();
    assert_eq!(applied.outcome, MergeApplied::Conflicted);
    assert_eq!(applied.inventory.entries.len(), 1);
    assert!(
        fs::read_to_string(f.0.join("file"))
            .unwrap()
            .contains("<<<<<<<")
    );
}

#[test]
fn many_regions_keep_character_coordinates_and_stable_distinct_identities() {
    let text = "λ\n<<<<<<< Current\ncurrent\n=======\nother\n>>>>>>> Other\n".repeat(2000);
    let regions = parse_conflict_regions(&text, 7).unwrap();
    assert_eq!(regions.len(), 2000);
    let chars: Vec<_> = text.chars().collect();
    for pair in regions.windows(2) {
        assert!(pair[0].range.end < pair[1].range.start);
        assert_ne!(pair[0].identity, pair[1].identity);
    }
    for region in regions {
        assert_eq!(
            chars[region.current].iter().collect::<String>(),
            "current\n"
        );
        assert_eq!(chars[region.other].iter().collect::<String>(), "other\n");
    }
}

#[cfg(unix)]
#[test]
fn whole_symlink_side_changes_link_without_following_target() {
    let f = Fixture::new();
    let external = f.0.join("external-target");
    fs::write(&external, "protected\n").unwrap();
    f.git(&["rm", "file"]);
    std::os::unix::fs::symlink("base-target", f.0.join("file")).unwrap();
    f.commit("base link");
    f.git(&["checkout", "-qb", "feature"]);
    fs::remove_file(f.0.join("file")).unwrap();
    std::os::unix::fs::symlink(&external, f.0.join("file")).unwrap();
    f.commit("other link");
    f.git(&["checkout", "-q", "main"]);
    fs::remove_file(f.0.join("file")).unwrap();
    std::os::unix::fs::symlink("current-target", f.0.join("file")).unwrap();
    f.commit("current link");
    let p = GitCliProvider::new("git");
    p.apply_merge(&f.repo(), &f.plan()).unwrap();
    let saved = p
        .prepare_resolution(
            &f.repo(),
            &f.0.join("file"),
            ResolutionChoice::SavedFile {
                allow_literal_markers: false,
            },
            BufferRevisionGuard::new(),
        )
        .unwrap();
    assert_eq!(
        saved.reviewed_content,
        BaseContent::Text("current-target".into())
    );
    let review = p
        .prepare_resolution(
            &f.repo(),
            &f.0.join("file"),
            ResolutionChoice::Other,
            BufferRevisionGuard::new(),
        )
        .unwrap();
    assert_eq!(
        review.reviewed_content,
        BaseContent::Text(external.to_str().unwrap().into())
    );
    p.resolve_conflict(&f.repo(), &review).unwrap();
    assert_eq!(fs::read_link(f.0.join("file")).unwrap(), external);
    assert_eq!(fs::read_to_string(&external).unwrap(), "protected\n");
}

#[test]
fn structural_rename_conflicts_expose_related_paths_and_refuse_partial_side_choice() {
    let f = Fixture::new();
    f.git(&["checkout", "-qb", "feature"]);
    f.git(&["mv", "file", "other-name"]);
    f.commit("other rename");
    f.git(&["checkout", "-q", "main"]);
    f.git(&["mv", "file", "current-name"]);
    f.commit("current rename");
    let p = GitCliProvider::new("git");
    let plan = f.plan();
    let MergePreviewOutcome::Merge {
        has_conflicts,
        messages,
        ..
    } = &plan.outcome
    else {
        panic!()
    };
    assert!(*has_conflicts);
    assert!(messages.iter().any(|m| m.kind.contains("rename")));
    let result = p.apply_merge(&f.repo(), &plan).unwrap();
    assert_eq!(result.outcome, MergeApplied::Conflicted);
    assert!(result.inventory.entries.len() >= 2);
    let paths = result
        .inventory
        .related_paths(&result.inventory.entries[0].path);
    assert!(paths.len() >= 2);
    for path in paths {
        assert!(
            p.prepare_resolution(
                &f.repo(),
                &f.0.join(path),
                ResolutionChoice::Other,
                BufferRevisionGuard::new()
            )
            .is_err()
        );
    }
}

#[test]
fn independent_modify_delete_conflicts_allow_independent_reviewed_deletions() {
    let f = Fixture::new();
    f.write("second", "second base\n");
    f.commit("second");
    f.git(&["checkout", "-qb", "feature"]);
    f.git(&["rm", "file", "second"]);
    f.commit("delete");
    f.git(&["checkout", "-q", "main"]);
    f.write("file", "file current\n");
    f.write("second", "second current\n");
    f.commit("modify both");
    let p = GitCliProvider::new("git");
    p.apply_merge(&f.repo(), &f.plan()).unwrap();
    let review = p
        .prepare_resolution(
            &f.repo(),
            &f.0.join("file"),
            ResolutionChoice::Other,
            BufferRevisionGuard::new(),
        )
        .unwrap();
    assert_eq!(
        p.resolve_conflict(&f.repo(), &review)
            .unwrap()
            .entries
            .len(),
        1
    );
    assert!(f.0.join("second").exists());
}

#[test]
fn attribute_changes_after_review_invalidate_even_without_config_change() {
    let f = Fixture::new();
    f.divergent(false);
    let p = GitCliProvider::new("git");
    let plan = f.plan();
    fs::create_dir_all(f.0.join(".git/info")).unwrap();
    f.write(".git/info/attributes", "* binary\n");
    assert!(p.apply_merge(&f.repo(), &plan).is_err());
    assert!(!f.0.join(".git/MERGE_HEAD").exists());
}

#[test]
fn whole_side_resolution_preserves_reviewed_executable_mode_when_filemode_is_disabled() {
    let f = Fixture::new();
    f.git(&["config", "core.filemode", "false"]);
    f.git(&["checkout", "-qb", "feature"]);
    f.write("file", "other\n");
    f.git(&["add", "file"]);
    f.git(&["update-index", "--chmod=+x", "file"]);
    f.commit("executable other");
    f.git(&["checkout", "-q", "main"]);
    f.write("file", "current\n");
    f.commit("current");
    let p = GitCliProvider::new("git");
    let applied = p.apply_merge(&f.repo(), &f.plan()).unwrap();
    let selected = applied.inventory.entries[0].other.as_ref().unwrap();
    assert_eq!(selected.mode, "100755");
    let review = p
        .prepare_resolution(
            &f.repo(),
            &f.0.join("file"),
            ResolutionChoice::Other,
            BufferRevisionGuard::new(),
        )
        .unwrap();
    assert!(
        p.resolve_conflict(&f.repo(), &review)
            .unwrap()
            .entries
            .is_empty()
    );
    assert_eq!(
        f.git(&["ls-files", "--stage", "file"]),
        format!("{} {} 0\tfile", selected.mode, selected.oid)
    );
    assert_eq!(fs::read_to_string(f.0.join("file")).unwrap(), "other\n");
}

#[cfg(unix)]
#[test]
fn whole_side_checkout_failure_preserves_unmerged_index() {
    let f = Fixture::new();
    f.divergent(true);
    let p = GitCliProvider::new("git");
    p.apply_merge(&f.repo(), &f.plan()).unwrap();
    let wrapper = f.0.join("failing-git");
    std::os::unix::fs::symlink(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/fixtures/stand-in"),
        &wrapper,
    )
    .unwrap();
    fs::write(
        wrapper.with_extension("behavior"),
        "for argument do if [ \"$argument\" = checkout ]; then exit 1; fi; done\nexec git \"$@\"\n",
    )
    .unwrap();
    f.write(".git/info/exclude", "failing-git\nfailing-git.behavior\n");
    let p = GitCliProvider::new(wrapper);
    let review = p
        .prepare_resolution(
            &f.repo(),
            &f.0.join("file"),
            ResolutionChoice::Other,
            BufferRevisionGuard::new(),
        )
        .unwrap();
    let index = fs::read(f.0.join(".git/index")).unwrap();
    let disk = fs::read(f.0.join("file")).unwrap();
    assert!(p.resolve_conflict(&f.repo(), &review).is_err());
    assert_eq!(fs::read(f.0.join(".git/index")).unwrap(), index);
    assert_eq!(fs::read(f.0.join("file")).unwrap(), disk);
    assert_eq!(p.conflicts(&f.repo()).unwrap().entries.len(), 1);
}

#[test]
fn literal_path_side_resolution_does_not_match_other_index_paths() {
    let f = Fixture::new();
    f.git(&["mv", "file", "[file]"]);
    f.write("f", "unrelated\n");
    f.commit("literal filename");
    f.git(&["checkout", "-qb", "feature"]);
    f.write("[file]", "other\n");
    f.commit("other");
    f.git(&["checkout", "-q", "main"]);
    f.write("[file]", "current\n");
    f.commit("current");
    let p = GitCliProvider::new("git");
    p.apply_merge(&f.repo(), &f.plan()).unwrap();
    let review = p
        .prepare_resolution(
            &f.repo(),
            &f.0.join("[file]"),
            ResolutionChoice::Other,
            BufferRevisionGuard::new(),
        )
        .unwrap();
    p.resolve_conflict(&f.repo(), &review).unwrap();
    assert_eq!(fs::read_to_string(f.0.join("[file]")).unwrap(), "other\n");
    assert_eq!(fs::read_to_string(f.0.join("f")).unwrap(), "unrelated\n");
}

#[test]
fn multiple_merge_bases_use_recursive_ancestor_consolidation_and_match_application() {
    let f = Fixture::new();
    f.git(&["branch", "feature"]);
    f.write("main-one", "A\n");
    f.commit("A");
    let a = f.git(&["rev-parse", "HEAD"]);
    f.git(&["checkout", "-q", "feature"]);
    f.write("feature-one", "B\n");
    f.commit("B");
    let b = f.git(&["rev-parse", "HEAD"]);
    f.git(&["checkout", "-q", "main"]);
    f.git(&["merge", "--no-ff", "-qm", "AB", &b]);
    f.git(&["checkout", "-q", "feature"]);
    f.git(&["merge", "--no-ff", "-qm", "BA", &a]);
    f.write("feature-two", "feature\n");
    f.commit("feature");
    f.git(&["checkout", "-q", "main"]);
    f.write("main-two", "main\n");
    f.commit("main");
    assert_eq!(
        f.git(&["merge-base", "--all", "main", "feature"])
            .lines()
            .count(),
        2
    );
    let p = GitCliProvider::new("git");
    let plan = f.plan();
    let MergePreviewOutcome::Merge {
        tree_oid,
        has_conflicts,
        ..
    } = &plan.outcome
    else {
        panic!()
    };
    assert!(!has_conflicts);
    p.apply_merge(&f.repo(), &plan).unwrap();
    assert_eq!(f.git(&["write-tree"]), *tree_oid);
}

#[test]
fn sha256_commit_and_tree_identities_are_kept_in_full() {
    let f = Fixture::with_format("sha256");
    f.divergent(false);
    let p = GitCliProvider::new("git");
    let plan = f.plan();
    assert_eq!(plan.destination_oid.len(), 64);
    assert_eq!(plan.source_oid.len(), 64);
    let MergePreviewOutcome::Merge { tree_oid, .. } = &plan.outcome else {
        panic!()
    };
    assert_eq!(tree_oid.len(), 64);
    p.apply_merge(&f.repo(), &plan).unwrap();
    assert_eq!(f.git(&["write-tree"]), *tree_oid);
}

#[test]
fn no_common_ancestor_has_an_actionable_refusal_and_never_changes_repository() {
    let f = Fixture::new();
    f.git(&["checkout", "--orphan", "feature"]);
    f.git(&["rm", "-rf", "."]);
    f.write("unrelated", "unrelated\n");
    f.commit("unrelated");
    f.git(&["checkout", "-q", "main"]);
    let p = GitCliProvider::new("git");
    let head = f.git(&["rev-parse", "HEAD"]);
    let error = p
        .prepare_merge(&f.repo(), "refs/heads/feature", BufferRevisionGuard::new())
        .unwrap_err();
    assert!(error.to_string().contains("no common ancestor"));
    assert_eq!(f.git(&["rev-parse", "HEAD"]), head);
    assert!(!f.0.join(".git/MERGE_HEAD").exists());
}

#[test]
fn equal_blob_content_conflicts_are_independent_for_whole_side_resolution() {
    let f = Fixture::new();
    f.write("second", "base\n");
    f.commit("same base");
    f.git(&["checkout", "-qb", "feature"]);
    f.write("file", "other\n");
    f.write("second", "other\n");
    f.commit("same other");
    f.git(&["checkout", "-q", "main"]);
    f.write("file", "current\n");
    f.write("second", "current\n");
    f.commit("same current");
    let p = GitCliProvider::new("git");
    let applied = p.apply_merge(&f.repo(), &f.plan()).unwrap();
    assert_eq!(
        applied
            .inventory
            .related_paths(std::path::Path::new("file"))
            .len(),
        1
    );
    let review = p
        .prepare_resolution(
            &f.repo(),
            &f.0.join("file"),
            ResolutionChoice::Current,
            BufferRevisionGuard::new(),
        )
        .unwrap();
    let inventory = p.resolve_conflict(&f.repo(), &review).unwrap();
    assert_eq!(inventory.entries.len(), 1);
    assert_eq!(inventory.entries[0].path, PathBuf::from("second"));
}

#[test]
fn public_plan_fields_cannot_substitute_an_unreviewed_action() {
    let f = Fixture::new();
    f.divergent(true);
    let p = GitCliProvider::new("git");
    let mut plan = f.plan();
    plan.outcome = MergePreviewOutcome::FastForward;
    assert!(p.apply_merge(&f.repo(), &plan).is_err());
    assert!(!f.0.join(".git/MERGE_HEAD").exists());
    p.apply_merge(&f.repo(), &f.plan()).unwrap();
    let mut review = p
        .prepare_resolution(
            &f.repo(),
            &f.0.join("file"),
            ResolutionChoice::Current,
            BufferRevisionGuard::new(),
        )
        .unwrap();
    review.choice = ResolutionChoice::Other;
    assert!(p.resolve_conflict(&f.repo(), &review).is_err());
    let mut completion = p
        .prepare_merge_completion(&f.repo(), BufferRevisionGuard::new())
        .unwrap();
    completion.message.push_str("changed outside review");
    assert!(p.abort_merge(&f.repo(), &completion).is_err());
}

#[test]
fn index_lock_failure_keeps_head_and_reports_actual_idle_repository() {
    let f = Fixture::new();
    f.divergent(false);
    let p = GitCliProvider::new("git");
    let plan = f.plan();
    let head = f.git(&["rev-parse", "HEAD"]);
    f.write(".git/index.lock", "fixture lock\n");
    assert!(p.apply_merge(&f.repo(), &plan).is_err());
    assert_eq!(f.git(&["rev-parse", "HEAD"]), head);
    assert_eq!(
        p.operation_state(&f.repo()).unwrap(),
        RepositoryOperation::Idle
    );
}

#[cfg(unix)]
#[test]
fn missing_merge_tree_capability_refuses_without_touching_index_or_disk() {
    let f = Fixture::new();
    f.divergent(false);
    let wrapper = f.0.join("old-git");
    std::os::unix::fs::symlink(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/fixtures/stand-in"),
        &wrapper,
    )
    .unwrap();
    fs::write(
        wrapper.with_extension("behavior"),
        "for argument do if [ \"$argument\" = merge-tree ]; then exit 129; fi; done\nexec git \"$@\"\n",
    )
    .unwrap();
    // Wrapper data belongs outside Git's tracked/untracked working tree.
    f.write(".git/info/exclude", "old-git\nold-git.behavior\n");
    let index = fs::read(f.0.join(".git/index")).unwrap();
    let error = GitCliProvider::new(wrapper)
        .prepare_merge(&f.repo(), "refs/heads/feature", BufferRevisionGuard::new())
        .unwrap_err();
    assert!(error.to_string().contains("upgrade Git"));
    assert_eq!(fs::read(f.0.join(".git/index")).unwrap(), index);
    assert_eq!(fs::read_to_string(f.0.join("file")).unwrap(), "current\n");
}

#[test]
fn ordinary_commit_cannot_complete_a_merge_started_after_message_capture() {
    let f = Fixture::new();
    f.divergent(false);
    let p = GitCliProvider::new("git");
    assert_eq!(
        p.operation_state(&f.repo()).unwrap(),
        RepositoryOperation::Idle
    );
    let captured_message = "ordinary message captured before external merge";
    f.git(&["merge", "--no-ff", "--no-commit", "feature"]);
    let head = f.git(&["rev-parse", "HEAD"]);
    let index = fs::read(f.0.join(".git/index")).unwrap();
    let error = p.commit(&f.repo(), captured_message).unwrap_err();
    assert!(error.to_string().contains("merge completion review"));
    assert_eq!(f.git(&["rev-parse", "HEAD"]), head);
    assert_eq!(fs::read(f.0.join(".git/index")).unwrap(), index);
    assert!(f.0.join(".git/MERGE_HEAD").exists());
    let review = p
        .prepare_merge_completion(&f.repo(), BufferRevisionGuard::new())
        .unwrap();
    p.commit_merge(&f.repo(), &review, "reviewed external merge")
        .unwrap();
    assert_eq!(
        f.git(&["rev-list", "--parents", "-n", "1", "HEAD"])
            .split_whitespace()
            .count(),
        3
    );

    f.write("ordinary", "ordinary staged edit\n");
    f.git(&["add", "ordinary"]);
    p.commit(&f.repo(), "ordinary commit after merge").unwrap();
    assert_eq!(
        f.git(&["rev-list", "--parents", "-n", "1", "HEAD"])
            .split_whitespace()
            .count(),
        2
    );
}

#[test]
fn unborn_checkout_conflict_inventory_is_empty_without_inventing_a_head() {
    let f = Fixture::new();
    f.git(&["symbolic-ref", "HEAD", "refs/heads/unborn"]);
    let inventory = GitCliProvider::new("git").conflicts(&f.repo()).unwrap();
    assert_eq!(inventory.operation, RepositoryOperation::Idle);
    assert!(inventory.entries.is_empty());
    assert_eq!(inventory.head_oid, None);
}
