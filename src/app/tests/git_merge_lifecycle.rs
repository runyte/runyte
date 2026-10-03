// SPDX-License-Identifier: MPL-2.0

use super::git_merges::MergeFixture;
use super::*;
use crate::{
    app::git_merges::ReviewPlan,
    git::{GitProvider, RepositoryOperation},
};
use std::sync::mpsc::Receiver;

fn reviewed_message(
    fixture: &MergeFixture,
    app: &mut App,
    operations: &Receiver<GitOperation>,
) -> usize {
    app.open_commit_message();
    let operation = operations.recv_timeout(Duration::from_secs(1)).unwrap();
    assert!(matches!(operation, GitOperation::Conflicts { .. }));
    let id = app.merge_ui.commit_preflight.as_ref().unwrap().0;
    app.apply_git_service_event(GitServiceEvent::Completed {
        id,
        operation,
        result: Box::new(Ok(GitResponse::Conflicts(
            fixture.provider().conflicts(&fixture.repository()).unwrap(),
        ))),
        state: GitServiceState::Completed,
        coalesced: false,
    });
    let GitOperation::PrepareMergeCompletion { repository, guard } =
        operations.recv_timeout(Duration::from_secs(1)).unwrap()
    else {
        panic!("active merge must request its own completion review")
    };
    let id = app.merge_ui.pending.as_ref().unwrap().id;
    let plan = fixture
        .provider()
        .prepare_merge_completion(&repository, guard)
        .unwrap();
    app.receive_merge_plan(
        Some(id),
        ReviewPlan::Completion(
            Box::new(plan),
            crate::app::git_merges::ReviewIntent::Continue,
        ),
    );
    assert!(app.merge_ui.review.is_some());
    assert!(!app.active_buffer().is_commit_message());
    app.approve_merge_review();
    let buffer = app.merge_ui.commit_buffer.unwrap();
    assert_eq!(app.active().buffer, buffer);
    assert!(app.buffers[buffer].is_commit_message());
    assert!(
        operations.try_recv().is_err(),
        "approval must not create the commit"
    );
    buffer
}

fn saved_merge_operation(
    app: &mut App,
    operations: &Receiver<GitOperation>,
    buffer: usize,
) -> GitMutation {
    app.commit_staged(buffer);
    let GitOperation::Mutate { mutation, .. } =
        operations.recv_timeout(Duration::from_secs(1)).unwrap()
    else {
        panic!("saving the reviewed message must submit a mutation")
    };
    assert!(matches!(mutation, GitMutation::CommitMerge { .. }));
    mutation
}

fn ordinary_message(app: &mut App, text: &str) -> usize {
    app.buffers.push(Buffer::commit_message(text));
    app.syntax.push(None);
    app.buffers.len() - 1
}

#[test]
fn merge_lifecycle_external_clean_and_empty_tree_merges_require_review_and_explicit_save() {
    for empty_tree in [false, true] {
        let fixture = MergeFixture::new(true, false);
        let before = fixture.git(&["rev-parse", "HEAD"]);
        if empty_tree {
            fixture.git(&["merge", "--no-ff", "--no-commit", "-s", "ours", "feature"]);
        } else {
            fixture.git(&["merge", "--no-ff", "--no-commit", "feature"]);
        }
        let (mut app, operations) = fixture.app();
        assert!(app.merge_ui.inventory.is_none());
        let buffer = reviewed_message(&fixture, &mut app, &operations);
        assert_eq!(
            app.merge_ui.commit.as_ref().unwrap().staged_diff.is_empty(),
            empty_tree
        );
        assert_eq!(fixture.git(&["rev-parse", "HEAD"]), before);
        let mutation = saved_merge_operation(&mut app, &operations, buffer);
        let GitMutation::CommitMerge { plan, message } = &mutation else {
            panic!()
        };
        let summary = fixture
            .provider()
            .commit_merge(&fixture.repository(), plan, message)
            .unwrap();
        app.apply_git_mutation_result_for_request(
            mutation,
            Vec::new(),
            Some(summary),
            None,
            (app.merge_ui.commit_request, GitServiceState::Completed),
            None,
        );
        assert!(app.closed_buffers.contains(&buffer));
        assert!(app.merge_ui.commit.is_none());
        assert_eq!(
            fixture
                .git(&["rev-list", "--parents", "-n", "1", "HEAD"])
                .split_whitespace()
                .count(),
            3
        );
        assert_eq!(
            fixture
                .provider()
                .operation_state(&fixture.repository())
                .unwrap(),
            RepositoryOperation::Idle
        );
    }
}

#[test]
fn merge_lifecycle_only_its_owned_message_can_save_and_success_closes_that_message() {
    let fixture = MergeFixture::new(true, false);
    fixture.git(&["merge", "--no-ff", "--no-commit", "feature"]);
    let (mut app, operations) = fixture.app();
    let ordinary = ordinary_message(&mut app, "earlier ordinary message\n");
    let buffer = reviewed_message(&fixture, &mut app, &operations);
    app.commit_staged(ordinary);
    assert!(operations.try_recv().is_err());
    assert!(app.status.contains("own reviewed merge-message"));
    assert_eq!(app.merge_ui.commit_buffer, Some(buffer));
    let mutation = saved_merge_operation(&mut app, &operations, buffer);
    let GitMutation::CommitMerge { plan, message } = &mutation else {
        panic!()
    };
    let summary = fixture
        .provider()
        .commit_merge(&fixture.repository(), plan, message)
        .unwrap();
    app.apply_git_mutation_result_for_request(
        mutation,
        Vec::new(),
        Some(summary),
        None,
        (app.merge_ui.commit_request, GitServiceState::Completed),
        None,
    );
    assert!(app.closed_buffers.contains(&buffer));
    assert!(!app.closed_buffers.contains(&ordinary));
    assert_eq!(
        app.buffers[ordinary].to_string(),
        "earlier ordinary message\n"
    );
    assert!(app.merge_ui.commit_buffer.is_none());
}

#[test]
fn merge_lifecycle_closing_the_message_revokes_unsubmitted_completion_authority() {
    let fixture = MergeFixture::new(true, false);
    fixture.git(&["merge", "--no-ff", "--no-commit", "feature"]);
    let (mut app, operations) = fixture.app();
    let buffer = reviewed_message(&fixture, &mut app, &operations);
    let captured = app.merge_ui.commit.as_ref().unwrap().clone();
    app.abandon_commit_message(buffer);
    assert!(app.closed_buffers.contains(&buffer));
    assert!(app.merge_ui.commit.is_none());
    assert!(app.merge_ui.commit_buffer.is_none());
    assert!(!app.commit_reviewed_merge(buffer, "late save".into()));
    assert!(operations.try_recv().is_err());
    assert!(
        fixture
            .provider()
            .commit_merge(&fixture.repository(), &captured, "late external use")
            .is_err()
    );
    assert!(fixture.0.join(".git/MERGE_HEAD").exists());
}

#[cfg(unix)]
#[test]
fn merge_lifecycle_failed_hook_retains_exact_message_and_review_for_successful_retry() {
    let fixture = MergeFixture::new(true, false);
    fixture.git(&["merge", "--no-ff", "--no-commit", "feature"]);
    let (mut app, operations) = fixture.app();
    let buffer = reviewed_message(&fixture, &mut app, &operations);
    let original_message = app.buffers[buffer].to_string();
    let hook = fixture.0.join(".git/hooks/pre-commit");
    std::os::unix::fs::symlink(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("src/fixtures/stand-in"),
        &hook,
    )
    .unwrap();
    fs::write(
        hook.with_extension("behavior"),
        "echo rejected >&2\nexit 1\n",
    )
    .unwrap();
    let mutation = saved_merge_operation(&mut app, &operations, buffer);
    let GitMutation::CommitMerge { plan, message } = &mutation else {
        panic!()
    };
    let error = fixture
        .provider()
        .commit_merge(&fixture.repository(), plan, message)
        .unwrap_err();
    app.apply_git_mutation_result_for_request(
        mutation,
        Vec::new(),
        None,
        Some(error),
        (app.merge_ui.commit_request, GitServiceState::Completed),
        None,
    );
    assert_eq!(app.merge_ui.commit_buffer, Some(buffer));
    assert!(app.merge_ui.commit.is_some());
    assert!(
        app.merge_ui
            .guards
            .iter()
            .any(|guard| guard.id() == app.merge_ui.commit.as_ref().unwrap().guard.id())
    );
    assert_eq!(app.active().buffer, buffer);
    assert!(!app.closed_buffers.contains(&buffer));
    assert_eq!(app.buffers[buffer].to_string(), original_message);
    assert!(fixture.0.join(".git/MERGE_HEAD").exists());
    while operations.try_recv().is_ok() {}
    fs::write(hook.with_extension("behavior"), "exit 0\n").unwrap();
    let mutation = saved_merge_operation(&mut app, &operations, buffer);
    let GitMutation::CommitMerge { plan, message } = &mutation else {
        panic!()
    };
    let summary = fixture
        .provider()
        .commit_merge(&fixture.repository(), plan, message)
        .unwrap();
    app.apply_git_mutation_result_for_request(
        mutation,
        Vec::new(),
        Some(summary),
        None,
        (app.merge_ui.commit_request, GitServiceState::Completed),
        None,
    );
    assert!(app.closed_buffers.contains(&buffer));
    assert!(app.merge_ui.commit.is_none());
    assert!(!fixture.0.join(".git/MERGE_HEAD").exists());
}

#[test]
fn merge_lifecycle_external_merge_after_ordinary_message_keeps_message_on_refusal() {
    let fixture = MergeFixture::new(true, false);
    let (mut app, operations) = fixture.app();
    let buffer = ordinary_message(&mut app, "ordinary message before external merge\n");
    app.active_mut().retarget(buffer);
    app.mode = Mode::Insert;
    fixture.git(&["merge", "--no-ff", "--no-commit", "feature"]);
    assert!(app.merge_ui.inventory.is_none());
    app.commit_staged(buffer);
    let GitOperation::Mutate { mutation, .. } =
        operations.recv_timeout(Duration::from_secs(1)).unwrap()
    else {
        panic!()
    };
    let GitMutation::Commit { message } = &mutation else {
        panic!()
    };
    let head = fixture.git(&["rev-parse", "HEAD"]);
    let error = fixture
        .provider()
        .commit(&fixture.repository(), message)
        .unwrap_err();
    app.apply_git_mutation_result(
        mutation,
        Vec::new(),
        None,
        Some(error),
        GitServiceState::Completed,
        None,
    );
    assert_eq!(app.active().buffer, buffer);
    assert!(!app.closed_buffers.contains(&buffer));
    assert_eq!(
        app.buffers[buffer].to_string(),
        "ordinary message before external merge\n"
    );
    assert_eq!(fixture.git(&["rev-parse", "HEAD"]), head);
    assert!(fixture.0.join(".git/MERGE_HEAD").exists());
}

#[cfg(unix)]
#[test]
fn merge_lifecycle_newly_opened_file_edit_after_hook_failure_revokes_retry() {
    let fixture = MergeFixture::new(true, false);
    fixture.git(&["merge", "--no-ff", "--no-commit", "feature"]);
    let (mut app, operations) = fixture.app();
    let buffer = reviewed_message(&fixture, &mut app, &operations);
    let hook = fixture.0.join(".git/hooks/pre-commit");
    std::os::unix::fs::symlink(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("src/fixtures/stand-in"),
        &hook,
    )
    .unwrap();
    fs::write(hook.with_extension("behavior"), "exit 1\n").unwrap();
    let mutation = saved_merge_operation(&mut app, &operations, buffer);
    let GitMutation::CommitMerge { plan, message } = &mutation else {
        panic!()
    };
    let captured = plan.clone();
    let error = fixture
        .provider()
        .commit_merge(&fixture.repository(), plan, message)
        .unwrap_err();
    app.apply_git_mutation_result(
        mutation,
        Vec::new(),
        None,
        Some(error),
        GitServiceState::Completed,
        None,
    );
    assert!(captured.guard.is_valid());
    app.open_file(fixture.0.join("other")).unwrap();
    app.mode = Mode::Insert;
    press(&mut app, 'x');
    assert!(app.active_buffer().dirty);
    assert_eq!(
        fs::read_to_string(fixture.0.join("other")).unwrap(),
        "other\n"
    );
    assert!(
        !captured.guard.is_valid(),
        "newly opened file edits must invalidate the retained retry authority"
    );
    fs::write(hook.with_extension("behavior"), "exit 0\n").unwrap();
    assert!(
        fixture
            .provider()
            .commit_merge(&fixture.repository(), &captured, "queued retry")
            .is_err()
    );
    assert_eq!(app.merge_ui.commit_buffer, Some(buffer));
    assert!(!app.closed_buffers.contains(&buffer));
    assert!(fixture.0.join(".git/MERGE_HEAD").exists());
}
