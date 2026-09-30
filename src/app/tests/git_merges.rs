// SPDX-License-Identifier: MPL-2.0

use super::*;
use crate::app::git_merges::ReviewPlan;
use crate::git::{
    BufferRevisionGuard, GitCliProvider, GitProvider, MergePreviewOutcome, ResolutionChoice,
};
use std::sync::mpsc::Receiver;

pub(super) struct MergeFixture(pub(super) PathBuf);
impl MergeFixture {
    pub(super) fn new(divergent: bool, conflict: bool) -> Self {
        let root = temporary("native-merge-ui");
        fs::create_dir_all(&root).unwrap();
        let fixture = Self(root.canonicalize().unwrap());
        fixture.git(&["init", "-q", "--initial-branch=main"]);
        for (key, value) in [
            ("user.name", "Runyte Test"),
            ("user.email", "runyte@example.invalid"),
            ("commit.gpgsign", "false"),
            ("core.autocrlf", "false"),
            ("core.hooksPath", ".git/hooks"),
        ] {
            fixture.git(&["config", key, value]);
        }
        fs::write(fixture.0.join("file"), "base\n").unwrap();
        fixture.commit("base");
        fixture.git(&["checkout", "-qb", "feature"]);
        fs::write(
            fixture.0.join(if conflict { "file" } else { "other" }),
            "other\n",
        )
        .unwrap();
        fixture.commit("feature");
        fixture.git(&["checkout", "-q", "main"]);
        if divergent {
            fs::write(fixture.0.join("file"), "current\n").unwrap();
            fixture.commit("current");
        }
        fixture
    }
    pub(super) fn git(&self, arguments: &[&str]) -> String {
        let output = std::process::Command::new("git")
            .args(arguments)
            .current_dir(&self.0)
            .env("XDG_CONFIG_HOME", self.0.join("test-config"))
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{arguments:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap().trim().into()
    }
    pub(super) fn commit(&self, message: &str) {
        self.git(&["add", "-A"]);
        self.git(&["commit", "-qm", message]);
    }
    pub(super) fn provider(&self) -> GitCliProvider {
        GitCliProvider::new("git")
    }
    pub(super) fn repository(&self) -> Repository {
        self.provider().discover(&self.0).unwrap().unwrap()
    }
    pub(super) fn app(&self) -> (App, Receiver<GitOperation>) {
        let mut ports = HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
            String::new(),
        )))));
        ports.replace_git(Box::new(
            crate::git::MemoryGitProvider::new(self.repository())
                .with_branches(&["main", "feature"], "main"),
        ));
        let mut app = App::new_in_isolated_project(&self.0, ports).unwrap();
        app.execute_command("git-branches").unwrap();
        let row = app
            .git_state
            .branch_rows()
            .iter()
            .position(|r| r.branch.as_ref().is_some_and(|b| b.name == "feature"))
            .unwrap();
        let offset = app.active_buffer().line_to_offset(row);
        app.active_mut().replace_selection(Selection::point(offset));
        let (service, operations) = GitServiceHandle::recording_for_test();
        app.attach_git_service(service);
        assert!(matches!(
            operations.recv_timeout(Duration::from_secs(1)).unwrap(),
            GitOperation::Discover { .. }
        ));
        app.git.attach(Some(self.repository()));
        (app, operations)
    }
    pub(super) fn review(&self, app: &mut App, operations: &Receiver<GitOperation>) {
        app.execute_command("git-merge-branch").unwrap();
        let operation = operations.recv_timeout(Duration::from_secs(1)).unwrap();
        let GitOperation::PrepareMerge {
            repository,
            source,
            guard,
        } = &operation
        else {
            panic!("expected prepare merge")
        };
        assert_eq!(source, "refs/heads/feature");
        let id = app.merge_ui.pending.as_ref().unwrap().id;
        let plan = self
            .provider()
            .prepare_merge(repository, source, guard.clone())
            .unwrap();
        app.apply_git_service_event(GitServiceEvent::Completed {
            id,
            operation,
            result: Box::new(Ok(GitResponse::PreparedMerge(Box::new(plan)))),
            state: GitServiceState::Completed,
            coalesced: false,
        });
        assert!(app.merge_ui.review.is_some());
    }
    pub(super) fn enter_conflicted_merge(&self, app: &mut App) {
        let repo = self.repository();
        let plan = self
            .provider()
            .prepare_merge(&repo, "refs/heads/feature", BufferRevisionGuard::new())
            .unwrap();
        let applied = self.provider().apply_merge(&repo, &plan).unwrap();
        app.show_git_conflicts(applied.inventory, true);
    }
}
impl Drop for MergeFixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn merge_ui_preview_predictions_cancel_and_duplicate_approval() {
    for (divergent, conflict, prediction) in [
        (false, false, "Fast-forward"),
        (true, false, "Clean merge"),
        (true, true, "conflict resolution"),
    ] {
        let fixture = MergeFixture::new(divergent, conflict);
        let original = fixture.git(&["rev-parse", "HEAD"]);
        let (mut app, operations) = fixture.app();
        fixture.review(&mut app, &operations);
        let snapshot = app.merge_review_snapshot().unwrap();
        assert!(snapshot.message.unwrap().contains(prediction));
        assert_eq!(
            snapshot.rows[snapshot.selected.unwrap()].identity,
            "cancel".into()
        );
        key(&mut app, KeyCode::Enter, Modifiers::NONE);
        assert!(app.merge_ui.review.is_none());
        assert_eq!(fixture.git(&["rev-parse", "HEAD"]), original);
        assert!(operations.try_recv().is_err());
        fixture.review(&mut app, &operations);
        press(&mut app, 'a');
        assert!(matches!(
            operations.recv_timeout(Duration::from_secs(1)).unwrap(),
            GitOperation::Mutate {
                mutation: GitMutation::Merge(_),
                ..
            }
        ));
        app.approve_merge_review();
        assert!(operations.try_recv().is_err());
    }
}

#[test]
fn merge_ui_noop_and_dirty_repository_disable_mutation() {
    let fixture = MergeFixture::new(false, false);
    fixture.git(&["merge", "--ff-only", "feature"]);
    let (mut app, operations) = fixture.app();
    fixture.review(&mut app, &operations);
    assert!(
        matches!(&app.merge_ui.review.as_ref().unwrap().plan, ReviewPlan::Merge(plan) if matches!(plan.outcome, MergePreviewOutcome::AlreadyContained))
    );
    assert!(!app.merge_ui.review.as_ref().unwrap().approval_available());
    app.approve_merge_review();
    assert!(operations.try_recv().is_err());
    app.cancel_merge_review();
    let branches = app.active().buffer;
    app.open_file(fixture.0.join("file")).unwrap();
    app.edit(Transaction::insert(0, "dirty\n"));
    app.active_mut().retarget(branches);
    let row = app
        .git_state
        .branch_rows()
        .iter()
        .position(|r| r.branch.as_ref().is_some_and(|b| b.name == "feature"))
        .unwrap();
    let offset = app.active_buffer().line_to_offset(row);
    app.active_mut().replace_selection(Selection::point(offset));
    context_action(&mut app, 'm');
    assert!(operations.try_recv().is_err());
    assert!(app.status.contains("save repository file buffers"));
}

#[test]
fn merge_ui_late_request_never_consumes_newer_pending_review() {
    let fixture = MergeFixture::new(true, false);
    let (mut app, operations) = fixture.app();
    app.prepare_branch_merge();
    let GitOperation::PrepareMerge {
        repository,
        source,
        guard,
    } = operations.recv_timeout(Duration::from_secs(1)).unwrap()
    else {
        panic!()
    };
    let old_id = app.merge_ui.pending.as_ref().unwrap().id;
    let old_plan = fixture
        .provider()
        .prepare_merge(&repository, &source, guard)
        .unwrap();
    app.prepare_branch_merge();
    let GitOperation::PrepareMerge {
        repository,
        source,
        guard,
    } = operations.recv_timeout(Duration::from_secs(1)).unwrap()
    else {
        panic!()
    };
    let new_id = app.merge_ui.pending.as_ref().unwrap().id;
    let new_plan = fixture
        .provider()
        .prepare_merge(&repository, &source, guard)
        .unwrap();
    app.receive_merge_plan(Some(old_id), ReviewPlan::Merge(Box::new(old_plan)));
    assert_eq!(app.merge_ui.pending.as_ref().unwrap().id, new_id);
    app.receive_merge_plan(Some(new_id), ReviewPlan::Merge(Box::new(new_plan)));
    assert!(app.merge_ui.review.is_some());
}

#[test]
fn merge_ui_navigation_detach_and_buffer_revisions_invalidate_authority() {
    let fixture = MergeFixture::new(true, false);
    let (mut app, operations) = fixture.app();
    app.open_file(fixture.0.join("file")).unwrap();
    let file = app.active().buffer;
    app.execute_command("git-branches").unwrap();
    let row = app
        .git_state
        .branch_rows()
        .iter()
        .position(|r| r.branch.as_ref().is_some_and(|b| b.name == "feature"))
        .unwrap();
    let offset = app.active_buffer().line_to_offset(row);
    app.active_mut().replace_selection(Selection::point(offset));
    fixture.review(&mut app, &operations);
    app.buffers[file].apply(&Transaction::insert(0, "edit"));
    assert!(!app.merge_ui.review.as_ref().unwrap().approval_available());
    app.cancel_merge_review();
    assert!(app.buffers[file].undo());
    app.buffers[file].save(false).unwrap();
    fixture.review(&mut app, &operations);
    app.active_mut().replace_selection(Selection::point(0));
    app.approve_merge_review();
    assert!(app.merge_ui.review.is_none());
    assert!(operations.try_recv().is_err());
    app.active_mut().replace_selection(Selection::point(offset));
    fixture.review(&mut app, &operations);
    app.persistent_session = true;
    app.request_detach();
    assert!(app.merge_ui.review.is_none());
}

#[test]
fn merge_ui_details_have_back_without_approval_and_drop_late_results() {
    let fixture = MergeFixture::new(true, false);
    let (mut app, operations) = fixture.app();
    fixture.review(&mut app, &operations);
    app.merge_ui.review.as_mut().unwrap().focus = 0;
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    let operation = operations.recv_timeout(Duration::from_secs(1)).unwrap();
    let id = app.merge_ui.details_request.unwrap();
    let GitOperation::RevisionFile {
        repository,
        comparison,
        file,
        split,
    } = &operation
    else {
        panic!()
    };
    let result = fixture
        .provider()
        .revision_file(repository, comparison, file, *split)
        .unwrap();
    assert_eq!(
        app.merge_review_snapshot()
            .unwrap()
            .rows
            .iter()
            .rev()
            .nth(1)
            .unwrap()
            .identity,
        "back".into()
    );
    press(&mut app, 'a');
    assert!(operations.try_recv().is_err());
    key(&mut app, KeyCode::Escape, Modifiers::NONE);
    assert!(app.merge_ui.review.as_ref().unwrap().detail.is_none());
    assert!(!app.receive_merge_detail(Some(id), result));
    assert_eq!(app.merge_ui.review.as_ref().unwrap().focus, 0);
}

#[test]
fn merge_ui_acknowledgment_treats_letters_as_text_and_registry_supports_chords() {
    let fixture = MergeFixture::new(true, false);
    let (mut app, operations) = fixture.app();
    fixture.review(&mut app, &operations);
    let review = app.merge_ui.review.as_mut().unwrap();
    review.requires_ack = true;
    review.focus = review.rows.len();
    for c in "ca".chars() {
        press(&mut app, c);
    }
    assert_eq!(app.merge_ui.review.as_ref().unwrap().acknowledgment, "ca");
    assert!(operations.try_recv().is_err());
    let review = app.merge_ui.review.as_mut().unwrap();
    review.acknowledgment = "main".into();
    review.cursor = 4;
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    let mut bindings = app
        .keymap
        .bindings()
        .iter()
        .filter(|b| b.target != BindingTarget::Editor(EditorCommand::MergeReviewApprove))
        .cloned()
        .collect::<Vec<_>>();
    bindings.push(Binding::implemented_in(
        &[Mode::Normal],
        BindingScope::GitMergeReview,
        [crate::keymap::Key::char('q'), crate::keymap::Key::char('q')],
        EditorCommand::MergeReviewApprove,
    ));
    app.keymap = crate::keymap::Keymap::new(bindings).unwrap().into();
    press(&mut app, 'q');
    assert!(operations.try_recv().is_err());
    press(&mut app, 'q');
    assert!(matches!(
        operations.recv_timeout(Duration::from_secs(1)).unwrap(),
        GitOperation::Mutate {
            mutation: GitMutation::Merge(_),
            ..
        }
    ));
}

#[test]
fn merge_ui_stale_inventory_preserves_new_request_and_does_not_steal_focus() {
    let fixture = MergeFixture::new(true, true);
    let (mut app, operations) = fixture.app();
    fixture.enter_conflicted_merge(&mut app);
    let original = app.merge_ui.inventory.clone().unwrap();
    app.open_git_conflicts();
    let _ = operations.recv_timeout(Duration::from_secs(1)).unwrap();
    let old = app.merge_ui.conflict_read.as_ref().unwrap().0;
    app.open_git_conflicts();
    let _ = operations.recv_timeout(Duration::from_secs(1)).unwrap();
    let new = app.merge_ui.conflict_read.as_ref().unwrap().0;
    app.receive_conflict_inventory(Some(old), original.clone());
    assert_eq!(app.merge_ui.conflict_read.as_ref().unwrap().0, new);
    app.execute_editor_command(EditorCommand::NewBuffer)
        .unwrap();
    let scratch = app.active().buffer;
    app.receive_conflict_inventory(Some(new), original.clone());
    assert_eq!(app.active().buffer, scratch);
    assert_eq!(app.merge_ui.inventory, Some(original));
}

#[test]
fn merge_ui_region_resolution_staging_preserves_undo_and_drops_resolved_row() {
    let fixture = MergeFixture::new(true, true);
    let (mut app, operations) = fixture.app();
    fixture.enter_conflicted_merge(&mut app);
    let row = app.active_buffer().line_to_offset(2);
    app.active_mut().replace_selection(Selection::point(row));
    app.open_git_conflict();
    let file = app.active().buffer;
    let original = app.active_buffer().to_string();
    app.choose_conflict_region(false);
    assert_eq!(app.active_buffer().to_string(), "current\n");
    app.buffers[file].save(false).unwrap();
    app.request_conflict_resolution(ResolutionChoice::SavedFile {
        allow_literal_markers: false,
    });
    let GitOperation::PrepareResolution {
        repository,
        path,
        choice,
        guard,
    } = operations.recv_timeout(Duration::from_secs(1)).unwrap()
    else {
        panic!()
    };
    let id = app.merge_ui.pending.as_ref().unwrap().id;
    let plan = fixture
        .provider()
        .prepare_resolution(&repository, &path, choice, guard)
        .unwrap();
    app.receive_merge_plan(Some(id), ReviewPlan::Resolution(Box::new(plan)));
    app.approve_merge_review();
    let GitOperation::Mutate { mutation, .. } =
        operations.recv_timeout(Duration::from_secs(1)).unwrap()
    else {
        panic!()
    };
    let GitMutation::ResolveConflict(plan) = &mutation else {
        panic!()
    };
    fixture
        .provider()
        .resolve_conflict(&repository, plan)
        .unwrap();
    app.apply_git_mutation_result(
        mutation,
        vec![path],
        Some("resolved".into()),
        None,
        GitServiceState::Completed,
        None,
    );
    let _ = operations.recv_timeout(Duration::from_secs(1)).unwrap();
    let id = app.merge_ui.conflict_read.as_ref().unwrap().0;
    app.receive_conflict_inventory(Some(id), fixture.provider().conflicts(&repository).unwrap());
    assert!(app.merge_ui.inventory.as_ref().unwrap().entries.is_empty());
    assert_eq!(app.key_binding_scope(), BindingScope::Global);
    assert!(app.buffers[file].undo());
    assert_eq!(app.buffers[file].to_string(), original);
    assert!(app.buffers[file].dirty);
    assert_eq!(fixture.git(&["show", ":file"]), "current");
    assert!(fixture.git(&["ls-files", "-u"]).is_empty());
}

#[test]
fn merge_ui_physical_branch_and_conflict_namespaces_use_registry() {
    let fixture = MergeFixture::new(true, true);
    let (mut app, operations) = fixture.app();
    let service = app.ports.git_service.take();
    press(&mut app, ' ');
    press(&mut app, 'g');
    press(&mut app, 'b');
    assert!(app.active_buffer().is_git_branches());
    app.ports.git_service = service;
    let row = app
        .git_state
        .branch_rows()
        .iter()
        .position(|r| r.branch.as_ref().is_some_and(|b| b.name == "feature"))
        .unwrap();
    let offset = app.active_buffer().line_to_offset(row);
    app.active_mut().replace_selection(Selection::point(offset));
    context_action(&mut app, 'm');
    assert!(matches!(
        operations.recv_timeout(Duration::from_secs(1)).unwrap(),
        GitOperation::PrepareMerge { .. }
    ));
    app.cancel_merge_review();
    press(&mut app, ' ');
    press(&mut app, 'g');
    press(&mut app, 'c');
    let operation = operations.recv_timeout(Duration::from_secs(1)).unwrap();
    assert!(matches!(operation, GitOperation::Conflicts { .. }));
    let id = app.merge_ui.conflict_read.as_ref().unwrap().0;
    app.receive_conflict_inventory(
        Some(id),
        fixture.provider().conflicts(&fixture.repository()).unwrap(),
    );
    assert!(app.active_buffer().is_git_conflicts());
    assert!(!app.command_capabilities().git_merge_active.is_available());
}

#[test]
fn merge_ui_backward_cross_file_navigation_selects_last_region() {
    let fixture = MergeFixture::new(true, true);
    let (mut app, _operations) = fixture.app();
    fixture.enter_conflicted_merge(&mut app);
    let marker = "<<<<<<< Current\ncurrent\n=======\nother\n>>>>>>> Other\n";
    fs::write(
        fixture.0.join("first"),
        format!("{marker}between\n{marker}"),
    )
    .unwrap();
    fs::write(fixture.0.join("last"), marker).unwrap();
    let mut inventory = app.merge_ui.inventory.clone().unwrap();
    let mut first = inventory.entries[0].clone();
    first.path = "first".into();
    let mut last = first.clone();
    last.path = "last".into();
    inventory.entries = vec![first, last];
    app.show_git_conflicts(inventory, false);
    app.open_file(fixture.0.join("last")).unwrap();
    app.active_mut().replace_selection(Selection::point(0));
    app.focus_conflict_region(false, true);
    assert_eq!(
        app.active_buffer().path.as_deref(),
        Some(fixture.0.join("first").as_path())
    );
    let regions =
        crate::git::conflict_regions::parse_conflict_regions(&app.active_buffer().to_string(), 7)
            .unwrap();
    assert_eq!(app.active().head(), regions.last().unwrap().range.start);
}
