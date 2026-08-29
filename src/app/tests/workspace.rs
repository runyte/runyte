// SPDX-License-Identifier: MPL-2.0

use super::*;

#[cfg(unix)]
fn open_session_manager_for_refresh(app: &mut App) {
    let mut picker = ListPicker::new("Sessions · loading…", Vec::new());
    picker.primary_action = Some("attach".to_owned());
    app.list = Some(picker);
}

#[test]
fn attach_alias_captures_the_editor_working_directory_for_relative_selectors() {
    let root = temporary("session-attach-working-directory");
    let editor_directory = root.join("nested");
    fs::create_dir_all(&editor_directory).unwrap();
    let editor_directory = editor_directory.canonicalize().unwrap();
    assert_ne!(std::env::current_dir().unwrap(), editor_directory);
    let mut app = App::new_in_isolated_project(
        &root,
        HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
            String::new(),
        ))))),
    )
    .unwrap();
    app.execute(crate::command::parse_named_command("cd", Some("nested")).unwrap())
        .unwrap();
    assert_eq!(app.working_directory, editor_directory);
    app.enable_persistent_session();

    app.execute(crate::command::parse_named_command("attach", Some("../project")).unwrap())
        .unwrap();

    assert_eq!(
        app.take_workspace_switch(),
        Some(WorkspaceSwitchRequest {
            selector: PathBuf::from("../project"),
            working_directory: editor_directory,
        })
    );
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn worktree_removal_refuses_unsaved_or_uninspectable_persistent_sessions() {
    let root = temporary("worktree-session-delete-guard");
    fs::create_dir_all(&root).unwrap();
    let root = root.canonicalize().unwrap();
    let target = root.join("linked");
    fs::create_dir_all(&target).unwrap();
    let target = target.canonicalize().unwrap();
    let mut app = App::new_in_isolated_project(
        &root,
        HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
            String::new(),
        ))))),
    )
    .unwrap();
    let plan = WorktreeRemovalPlan {
        path: target.clone(),
        head: Some("0123456789abcdef".to_owned()),
        branch: Some("feature".to_owned()),
        upstream: None,
        detached_retained: false,
        required_authorization: DeletionAuthorization::Enter,
    };
    let row = |unsaved_buffers| WorkspaceRow {
        id: "linked".to_owned(),
        name: None,
        number: None,
        last_active_unix_seconds: None,
        project_root: target.clone(),
        running: true,
        incompatible_protocol: None,
        unsaved_buffers,
        pending_wait_requests: Some(0),
        live_terminals: Some(0),
        terminal_sessions: Some(0),
        interactive_attached: Some(false),
        open_buffers: None,
        git: None,
        missing_directory: false,
    };

    app.worktree_removal_generation = 1;
    app.pending_worktree_removal = Some(PendingWorktreeRemovalCheck {
        branch: None,
        plan: plan.clone(),
        authorization: None,
        origin: None,
    });
    app.finish_worktree_session_check(1, target.clone(), Ok(Some(row(Some(2)))));
    assert!(app.status_error);
    assert!(
        app.status.contains("2 unsaved file buffers"),
        "{}",
        app.status
    );
    assert!(app.git_worktree_removal.is_none());

    app.worktree_removal_generation = 2;
    app.pending_worktree_removal = Some(PendingWorktreeRemovalCheck {
        branch: None,
        plan: plan.clone(),
        authorization: None,
        origin: None,
    });
    app.finish_worktree_session_check(2, target.clone(), Ok(Some(row(None))));
    assert!(app.status_error);
    assert!(
        app.status.contains("unsaved state is unavailable"),
        "{}",
        app.status
    );

    app.worktree_removal_generation = 3;
    app.pending_worktree_removal = Some(PendingWorktreeRemovalCheck {
        branch: None,
        plan,
        authorization: None,
        origin: None,
    });
    app.finish_worktree_session_check(3, target.clone(), Ok(Some(row(Some(0)))));
    assert!(app.git_worktree_removal.is_some());

    fs::remove_dir_all(root).unwrap();
}

/// A clean session on the worktree does not refuse the removal — it goes with
/// it. The compound action is named in the confirmation and raises the bar to
/// typed text, whatever the worktree's own Git state would have settled for.
#[cfg(unix)]
#[test]
fn worktree_removal_names_its_session_and_takes_it_down_before_the_directory() {
    let root = temporary("worktree-session-cascade");
    fs::create_dir_all(&root).unwrap();
    let root = root.canonicalize().unwrap();
    let target = root.join("linked");
    fs::create_dir_all(&target).unwrap();
    let target = target.canonicalize().unwrap();
    let mut app = App::new_in_isolated_project(
        &root,
        HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
            String::new(),
        ))))),
    )
    .unwrap();
    app.enable_persistent_session();
    let plan = WorktreeRemovalPlan {
        path: target.clone(),
        head: Some("0123456789abcdef".to_owned()),
        branch: Some("feature".to_owned()),
        upstream: None,
        // A clean worktree on a retained branch: Enter alone would have done.
        required_authorization: DeletionAuthorization::Enter,
        detached_retained: false,
    };
    app.worktree_removal_generation = 1;
    app.pending_worktree_removal = Some(PendingWorktreeRemovalCheck {
        branch: None,
        plan,
        authorization: None,
        origin: None,
    });
    app.finish_worktree_session_check(
        1,
        target.clone(),
        Ok(Some(WorkspaceRow {
            id: "linked".to_owned(),
            name: Some("runyte-feature".to_owned()),
            number: Some(5),
            last_active_unix_seconds: None,
            project_root: target.clone(),
            running: true,
            incompatible_protocol: None,
            unsaved_buffers: Some(0),
            pending_wait_requests: Some(0),
            live_terminals: Some(0),
            terminal_sessions: Some(0),
            interactive_attached: Some(false),
            open_buffers: Some(3),
            git: None,
            missing_directory: false,
        })),
    );

    let confirmation = app
        .git_worktree_removal
        .as_ref()
        .expect("a clean session should be offered rather than refused");
    let message = confirmation.message();
    assert!(
        message.contains("This also stops and forgets session 5 (runyte-feature)."),
        "{message}"
    );
    // The session is what raises this from an Enter confirmation to a typed
    // one; stopping somebody's running editor is not a one-keystroke decision.
    assert!(
        message.contains("Type feature exactly to continue."),
        "{message}"
    );

    fs::remove_dir_all(root).unwrap();
}

/// The order is the whole point: the host owns the worktree directory, so Git
/// is asked to remove it only after the stop has reported.
#[cfg(unix)]
#[test]
fn a_confirmed_worktree_removal_waits_for_its_session_to_stop_before_removing_it() {
    use crate::git::{MemoryGitProvider, Repository};

    let root = temporary("worktree-session-order");
    fs::create_dir_all(&root).unwrap();
    let root = root.canonicalize().unwrap();
    let target = root.join("linked");
    fs::create_dir_all(&target).unwrap();
    let target = target.canonicalize().unwrap();
    let mut ports = HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
        String::new(),
    )))));
    let provider = Rc::new(
        MemoryGitProvider::new(Repository::new(&root)).with_worktrees(vec![test_worktree(
            target.clone(),
            "feature",
            &root,
        )]),
    );
    ports.replace_git(Box::new(Rc::clone(&provider)));
    let mut app = App::new_in_isolated_project(&root, ports).unwrap();
    app.enable_persistent_session();
    app.execute_command("git-worktrees").unwrap();
    // The guarded removal re-prepares and compares its plan, so the teardown
    // has to be holding the one Git would answer with now.
    let plan = crate::git::GitProvider::prepare_worktree_removal(
        provider.as_ref(),
        &Repository::new(&root),
        &target,
    )
    .unwrap();

    // The confirmation has been accepted and the stop asked for; this is the
    // state the cascade waits in.
    app.worktree_teardown = Some(WorktreeTeardown {
        plan,
        authorization: DeletionAuthorization::Typed,
        session: Some(AttachedSession {
            name: "runyte-feature".to_owned(),
            number: Some(5),
            root: target.clone(),
        }),
        root: target.clone(),
        branch: None,
        workspace_request_generation: Some(app.workspace_generation),
        git_request: None,
        stage: WorktreeTeardownStage::Stopping,
    });
    assert!(
        provider.removed_worktrees().is_empty(),
        "the directory went before its host did"
    );

    let generation = app.workspace_generation;
    // An unrelated session-list refresh may be requested while the host is
    // stopping. Its newer generation must not orphan this teardown reply.
    app.workspace_generation = app.workspace_generation.wrapping_add(1);
    app.apply_workspace_event(WorkspaceEvent::Stopped {
        generation,
        selector: target.clone(),
        result: Ok(()),
    });

    assert_eq!(
        provider.removed_worktrees(),
        vec![target.clone()],
        "status: {}",
        app.status
    );
    assert!(
        app.status
            .contains("and stopped session 5 (runyte-feature)"),
        "{}",
        app.status
    );

    fs::remove_dir_all(root).unwrap();
}

/// A stop that fails leaves everything standing. The directory is still the
/// host's, so removing it anyway would be the one outcome worse than refusing.
#[cfg(unix)]
#[test]
fn a_session_that_will_not_stop_leaves_its_worktree_alone() {
    use crate::git::{MemoryGitProvider, Repository};

    let root = temporary("worktree-session-stop-failure");
    fs::create_dir_all(&root).unwrap();
    let root = root.canonicalize().unwrap();
    let target = root.join("linked");
    fs::create_dir_all(&target).unwrap();
    let target = target.canonicalize().unwrap();
    let mut ports = HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
        String::new(),
    )))));
    let provider = Rc::new(
        MemoryGitProvider::new(Repository::new(&root)).with_worktrees(vec![test_worktree(
            target.clone(),
            "feature",
            &root,
        )]),
    );
    ports.replace_git(Box::new(Rc::clone(&provider)));
    let mut app = App::new_in_isolated_project(&root, ports).unwrap();
    app.enable_persistent_session();
    app.execute_command("git-worktrees").unwrap();
    app.worktree_teardown = Some(WorktreeTeardown {
        plan: crate::git::GitProvider::prepare_worktree_removal(
            provider.as_ref(),
            &Repository::new(&root),
            &target,
        )
        .unwrap(),
        authorization: DeletionAuthorization::Typed,
        session: Some(AttachedSession {
            name: "runyte-feature".to_owned(),
            number: None,
            root: target.clone(),
        }),
        root: target.clone(),
        branch: None,
        workspace_request_generation: Some(app.workspace_generation),
        git_request: None,
        stage: WorktreeTeardownStage::Stopping,
    });

    let generation = app.workspace_generation;
    app.apply_workspace_event(WorkspaceEvent::Stopped {
        generation,
        selector: target.clone(),
        result: Err("a live terminal child is running".to_owned()),
    });

    assert!(app.status_error);
    assert!(
        app.status.contains("could not be stopped"),
        "{}",
        app.status
    );
    assert!(provider.removed_worktrees().is_empty());

    fs::remove_dir_all(root).unwrap();
}

/// With the Git service attached, `apply_guarded_worktree_removal` only queues
/// the mutation: its guarded re-check runs later and can still refuse. Nothing
/// below the worktree may happen until Git has said the directory is gone.
#[cfg(unix)]
#[test]
fn an_asynchronous_removal_takes_nothing_further_down_until_git_reports_success() {
    use crate::git::{
        GitMutation, GitServiceHandle, GitServiceState, MemoryGitProvider, Repository,
    };

    let run = |succeeds: bool| {
        let root = temporary(if succeeds {
            "async-removal-ok"
        } else {
            "async-removal-refused"
        });
        fs::create_dir_all(&root).unwrap();
        let root = root.canonicalize().unwrap();
        let target = root.join("linked");
        fs::create_dir_all(&target).unwrap();
        let target = target.canonicalize().unwrap();
        let mut ports = HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
            String::new(),
        )))));
        let provider = Rc::new(
            MemoryGitProvider::new(Repository::new(&root))
                .with_branches(&["feature", "main"], "main")
                .with_worktrees(vec![test_worktree(target.clone(), "feature", &root)]),
        );
        ports.replace_git(Box::new(Rc::clone(&provider)));
        let mut app = App::new_in_isolated_project(&root, ports).unwrap();
        app.enable_persistent_session();
        app.execute_command("git-worktrees").unwrap();
        let (service, operations) = GitServiceHandle::recording_for_test();
        app.attach_git_service(service);

        let plan = crate::git::GitProvider::prepare_worktree_removal(
            provider.as_ref(),
            &Repository::new(&root),
            &target,
        )
        .unwrap();
        let branch = crate::git::GitProvider::prepare_branch_deletion(
            provider.as_ref(),
            &Repository::new(&root),
            "feature",
        )
        .unwrap();
        app.worktree_teardown = Some(WorktreeTeardown {
            plan: plan.clone(),
            authorization: DeletionAuthorization::Typed,
            session: None,
            root: target.clone(),
            branch: Some(branch),
            workspace_request_generation: Some(app.workspace_generation),
            git_request: None,
            stage: WorktreeTeardownStage::Stopping,
        });

        let generation = app.workspace_generation;
        app.apply_workspace_event(WorkspaceEvent::Stopped {
            generation,
            selector: target.clone(),
            result: Ok(()),
        });

        // The removal was handed to the service, not performed, and the
        // cascade is waiting on it rather than treating the hand-off as done.
        // Attaching the service also queues discovery and refresh work, so the
        // mutation is looked for among everything sent rather than assumed to
        // be first.
        let drain = |operations: &std::sync::mpsc::Receiver<crate::git::GitOperation>| {
            let mut sent = Vec::new();
            while let Ok(operation) = operations.recv_timeout(std::time::Duration::from_millis(250))
            {
                sent.push(operation);
            }
            sent
        };
        assert!(
            drain(&operations).iter().any(|operation| matches!(
                operation,
                crate::git::GitOperation::Mutate {
                    mutation: GitMutation::RemoveWorktree { .. },
                    ..
                }
            )),
            "the removal should have been queued"
        );
        assert_eq!(
            app.worktree_teardown
                .as_ref()
                .map(|teardown| teardown.stage),
            Some(WorktreeTeardownStage::Removing)
        );
        assert!(
            provider.deletions().is_empty(),
            "the branch went before the worktree was known to be gone"
        );

        let mutation = GitMutation::RemoveWorktree {
            plan: Box::new(plan),
            authorization: DeletionAuthorization::Typed,
        };
        let request = app
            .worktree_teardown
            .as_ref()
            .and_then(|teardown| teardown.git_request);
        app.apply_git_mutation_result_for_request(
            mutation,
            Vec::new(),
            None,
            (!succeeds).then(|| crate::git::GitError::Failed {
                command: "git worktree remove".to_owned(),
                code: Some(1),
                stderr: "the worktree changed after it was reviewed".to_owned(),
            }),
            (request, GitServiceState::Completed),
            None,
        );

        // The branch deletion, if it happened at all, went to the service too.
        let deleted_branch = drain(&operations).iter().any(|operation| {
            matches!(
                operation,
                crate::git::GitOperation::Mutate {
                    mutation: GitMutation::DeleteBranch { .. },
                    ..
                }
            )
        });
        let status = app.status.clone();
        let teardown = app.worktree_teardown.is_some();
        fs::remove_dir_all(root).unwrap();
        (deleted_branch, status, teardown)
    };

    // A refused removal stops the cascade there: the directory is still
    // present, so the branch checked out in it must not be deleted.
    let (deleted_branch, status, teardown) = run(false);
    assert!(
        !deleted_branch,
        "the branch went down with a failed removal"
    );
    assert!(
        !teardown,
        "the abandoned cascade should not still be pending"
    );
    assert!(status.contains("changed after it was reviewed"), "{status}");

    // A successful one carries on into the branch above it.
    let (deleted_branch, _status, teardown) = run(true);
    assert!(deleted_branch, "a successful removal must reach its branch");
    assert!(
        teardown,
        "the cascade must keep owning the queued branch deletion"
    );
}

/// A worktree with no running session skips the stop stage, but it still has
/// to wait in the removal stage while the production Git service performs the
/// guarded mutation. Marking it as already forgetting would make the matching
/// completion invisible and strand both the history record and branch.
#[cfg(unix)]
#[test]
fn an_asynchronous_removal_without_a_session_waits_in_the_removal_stage() {
    use crate::git::{GitMutation, GitServiceHandle, MemoryGitProvider, Repository};

    let root = temporary("async-removal-no-session");
    fs::create_dir_all(&root).unwrap();
    let root = root.canonicalize().unwrap();
    let target = root.join("linked");
    fs::create_dir_all(&target).unwrap();
    let target = target.canonicalize().unwrap();
    let mut ports = HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
        String::new(),
    )))));
    let provider = Rc::new(
        MemoryGitProvider::new(Repository::new(&root))
            .with_branches(&["feature", "main"], "main")
            .with_worktrees(vec![test_worktree(target.clone(), "feature", &root)]),
    );
    ports.replace_git(Box::new(Rc::clone(&provider)));
    let mut app = App::new_in_isolated_project(&root, ports).unwrap();
    app.execute_command("git-worktrees").unwrap();
    let (service, operations) = GitServiceHandle::recording_for_test();
    app.attach_git_service(service);

    let plan = crate::git::GitProvider::prepare_worktree_removal(
        provider.as_ref(),
        &Repository::new(&root),
        &target,
    )
    .unwrap();
    let branch = crate::git::GitProvider::prepare_branch_deletion(
        provider.as_ref(),
        &Repository::new(&root),
        "feature",
    )
    .unwrap();
    app.begin_worktree_teardown(plan, DeletionAuthorization::Typed, None, Some(branch));

    assert_eq!(
        app.worktree_teardown
            .as_ref()
            .map(|teardown| teardown.stage),
        Some(WorktreeTeardownStage::Removing)
    );
    let mut removal_queued = false;
    while let Ok(operation) = operations.recv_timeout(std::time::Duration::from_millis(250)) {
        if matches!(
            operation,
            crate::git::GitOperation::Mutate {
                mutation: GitMutation::RemoveWorktree { .. },
                ..
            }
        ) {
            removal_queued = true;
        }
    }
    assert!(
        removal_queued,
        "the guarded removal should have been queued"
    );

    fs::remove_dir_all(root).unwrap();
}

/// The catalog forget is part of the teardown even when a manager refresh is
/// requested after it. Its own generation must remain sufficient to finish
/// the cascade instead of being discarded as an obsolete list request.
#[cfg(unix)]
#[test]
fn a_teardown_forget_reply_survives_a_newer_workspace_refresh() {
    use crate::git::{MemoryGitProvider, Repository};

    let root = temporary("teardown-forget-generation");
    fs::create_dir_all(&root).unwrap();
    let root = root.canonicalize().unwrap();
    let target = root.join("linked");
    fs::create_dir_all(&target).unwrap();
    let target = target.canonicalize().unwrap();
    let provider = MemoryGitProvider::new(Repository::new(&root))
        .with_worktrees(vec![test_worktree(target.clone(), "feature", &root)]);
    let mut app = App::new_in_isolated_project(
        &root,
        HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
            String::new(),
        ))))),
    )
    .unwrap();
    let request_generation = 7;
    app.workspace_generation = request_generation + 1;
    app.worktree_teardown = Some(WorktreeTeardown {
        plan: crate::git::GitProvider::prepare_worktree_removal(
            &provider,
            &Repository::new(&root),
            &target,
        )
        .unwrap(),
        authorization: DeletionAuthorization::Typed,
        session: None,
        root: target.clone(),
        branch: None,
        workspace_request_generation: Some(request_generation),
        git_request: None,
        stage: WorktreeTeardownStage::Forgetting,
    });

    app.apply_workspace_event(WorkspaceEvent::Forgotten {
        generation: request_generation,
        path: target,
        result: Ok(false),
    });

    assert!(
        app.worktree_teardown.is_none(),
        "the newer refresh must not orphan the completed teardown"
    );
    assert!(app.status.contains("removed worktree"), "{}", app.status);
    fs::remove_dir_all(root).unwrap();
}

/// Forgetting the catalog record is a real cascade level. Once it fails the
/// removed directory can only be reported as partial success; the branch
/// above it must remain intact.
#[cfg(unix)]
#[test]
fn a_failed_teardown_forget_leaves_the_branch_intact() {
    use crate::git::{MemoryGitProvider, Repository};

    let root = temporary("teardown-forget-failure");
    fs::create_dir_all(&root).unwrap();
    let root = root.canonicalize().unwrap();
    let target = root.join("linked");
    fs::create_dir_all(&target).unwrap();
    let target = target.canonicalize().unwrap();
    let provider = Rc::new(
        MemoryGitProvider::new(Repository::new(&root))
            .with_branches(&["feature", "main"], "main")
            .with_worktrees(vec![test_worktree(target.clone(), "feature", &root)]),
    );
    let mut ports = HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
        String::new(),
    )))));
    ports.replace_git(Box::new(Rc::clone(&provider)));
    let mut app = App::new_in_isolated_project(&root, ports).unwrap();
    let request_generation = 9;
    app.workspace_generation = request_generation;
    app.worktree_teardown = Some(WorktreeTeardown {
        plan: crate::git::GitProvider::prepare_worktree_removal(
            provider.as_ref(),
            &Repository::new(&root),
            &target,
        )
        .unwrap(),
        authorization: DeletionAuthorization::Typed,
        session: None,
        root: target.clone(),
        branch: Some(
            crate::git::GitProvider::prepare_branch_deletion(
                provider.as_ref(),
                &Repository::new(&root),
                "feature",
            )
            .unwrap(),
        ),
        workspace_request_generation: Some(request_generation),
        git_request: None,
        stage: WorktreeTeardownStage::Forgetting,
    });

    app.apply_workspace_event(WorkspaceEvent::Forgotten {
        generation: request_generation,
        path: target,
        result: Err("catalog is read-only".to_owned()),
    });

    assert!(app.worktree_teardown.is_none());
    assert!(provider.deletions().is_empty());
    assert!(
        app.status.contains("catalog is read-only"),
        "{}",
        app.status
    );
    assert!(
        app.status.contains("branch feature was not deleted"),
        "{}",
        app.status
    );
    fs::remove_dir_all(root).unwrap();
}

/// Only one destructive teardown may own the service replies at a time. A
/// second confirmation is refused instead of replacing the first cascade's
/// path and request identities.
#[cfg(unix)]
#[test]
fn a_second_worktree_teardown_cannot_replace_the_first() {
    use crate::git::{MemoryGitProvider, Repository};

    let root = temporary("serialized-worktree-teardown");
    fs::create_dir_all(&root).unwrap();
    let root = root.canonicalize().unwrap();
    let first = root.join("first");
    let second = root.join("second");
    fs::create_dir_all(&first).unwrap();
    fs::create_dir_all(&second).unwrap();
    let first = first.canonicalize().unwrap();
    let second = second.canonicalize().unwrap();
    let provider = Rc::new(
        MemoryGitProvider::new(Repository::new(&root)).with_worktrees(vec![
            test_worktree(first.clone(), "first", &root),
            test_worktree(second.clone(), "second", &root),
        ]),
    );
    let mut ports = HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
        String::new(),
    )))));
    ports.replace_git(Box::new(Rc::clone(&provider)));
    let mut app = App::new_in_isolated_project(&root, ports).unwrap();
    let first_plan = crate::git::GitProvider::prepare_worktree_removal(
        provider.as_ref(),
        &Repository::new(&root),
        &first,
    )
    .unwrap();
    let second_plan = crate::git::GitProvider::prepare_worktree_removal(
        provider.as_ref(),
        &Repository::new(&root),
        &second,
    )
    .unwrap();
    app.worktree_teardown = Some(WorktreeTeardown {
        plan: first_plan,
        authorization: DeletionAuthorization::Typed,
        session: None,
        root: first.clone(),
        branch: None,
        workspace_request_generation: None,
        git_request: None,
        stage: WorktreeTeardownStage::Removing,
    });

    app.begin_worktree_teardown(second_plan, DeletionAuthorization::Typed, None, None);

    assert!(app.status_error);
    assert!(app.status.contains("still in progress"), "{}", app.status);
    assert_eq!(
        app.worktree_teardown
            .as_ref()
            .map(|teardown| teardown.plan.path.as_path()),
        Some(first.as_path())
    );
    assert!(provider.removed_worktrees().is_empty());
    fs::remove_dir_all(root).unwrap();
}

/// A confirmed deletion owns its second session inspection. A new preflight
/// cannot replace that pending check and make the accepted action disappear.
#[cfg(unix)]
#[test]
fn a_second_session_check_cannot_replace_a_confirmed_worktree_removal() {
    let root = temporary("serialized-worktree-session-check");
    fs::create_dir_all(&root).unwrap();
    let first = root.join("first");
    let second = root.join("second");
    let plan = |path: PathBuf| WorktreeRemovalPlan {
        path,
        head: Some("0123456789abcdef".to_owned()),
        branch: Some("feature".to_owned()),
        upstream: None,
        detached_retained: false,
        required_authorization: DeletionAuthorization::Typed,
    };
    let first_plan = plan(first.clone());
    let mut app = App::new_in_isolated_project(
        &root,
        HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
            String::new(),
        ))))),
    )
    .unwrap();
    app.worktree_removal_generation = 11;
    app.pending_worktree_removal = Some(PendingWorktreeRemovalCheck {
        plan: first_plan,
        authorization: Some(DeletionAuthorization::Typed),
        origin: None,
        branch: None,
    });

    assert!(app.request_worktree_session_check(
        plan(second),
        Some(DeletionAuthorization::Typed),
        None,
    ));

    assert_eq!(app.worktree_removal_generation, 11);
    assert_eq!(
        app.pending_worktree_removal
            .as_ref()
            .map(|pending| pending.plan.path.as_path()),
        Some(first.as_path())
    );
    assert!(app.status_error);
    assert!(app.status.contains("still in progress"), "{}", app.status);
    fs::remove_dir_all(root).unwrap();
}

/// The final branch mutation remains part of the serialized cascade until its
/// exact request reports. Its completion restores the full compound summary.
#[cfg(unix)]
#[test]
fn a_cascade_owns_its_final_branch_request_until_completion() {
    use crate::git::{
        GitMutation, GitServiceHandle, GitServiceState, MemoryGitProvider, Repository,
    };

    let root = temporary("serialized-final-branch");
    fs::create_dir_all(&root).unwrap();
    let root = root.canonicalize().unwrap();
    let target = root.join("removed-linked");
    let repository = Repository::new(&root);
    let provider = Rc::new(
        MemoryGitProvider::new(repository.clone()).with_branches(&["feature", "main"], "main"),
    );
    let mut ports = HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
        String::new(),
    )))));
    ports.replace_git(Box::new(Rc::clone(&provider)));
    let mut app = App::new_in_isolated_project(&root, ports).unwrap();
    app.git.attach(Some(repository.clone()));
    let (service, _operations) = GitServiceHandle::recording_for_test();
    app.attach_git_service(service);
    let branch =
        crate::git::GitProvider::prepare_branch_deletion(provider.as_ref(), &repository, "feature")
            .unwrap();
    app.worktree_teardown = Some(WorktreeTeardown {
        plan: WorktreeRemovalPlan {
            path: target.clone(),
            head: Some("0123456789abcdef".to_owned()),
            branch: Some("feature".to_owned()),
            upstream: None,
            detached_retained: false,
            required_authorization: DeletionAuthorization::Typed,
        },
        authorization: DeletionAuthorization::Typed,
        session: None,
        root: target.clone(),
        branch: Some(branch.clone()),
        workspace_request_generation: None,
        git_request: None,
        stage: WorktreeTeardownStage::Forgetting,
    });

    app.finish_worktree_teardown();

    let request = app
        .worktree_teardown
        .as_ref()
        .and_then(|teardown| teardown.git_request)
        .expect("the final branch request remains correlated");
    assert_eq!(
        app.worktree_teardown
            .as_ref()
            .map(|teardown| teardown.stage),
        Some(WorktreeTeardownStage::BranchDeleting)
    );
    app.apply_git_mutation_result_for_request(
        GitMutation::DeleteBranch {
            plan: Box::new(branch.clone()),
            authorization: DeletionAuthorization::Typed,
        },
        Vec::new(),
        None,
        None,
        (
            Some(crate::git::GitRequestId::from_raw(request.get() + 100)),
            GitServiceState::Completed,
        ),
        None,
    );
    assert!(
        app.worktree_teardown.is_some(),
        "a different branch request must not complete this cascade"
    );
    app.apply_git_mutation_result_for_request(
        GitMutation::DeleteBranch {
            plan: Box::new(branch),
            authorization: DeletionAuthorization::Typed,
        },
        Vec::new(),
        Some("provider summary must not hide the compound result".to_owned()),
        None,
        (Some(request), GitServiceState::Completed),
        None,
    );

    assert!(app.worktree_teardown.is_none());
    assert!(
        app.status
            .contains("deleted branch feature, removed worktree"),
        "{}",
        app.status
    );
    fs::remove_dir_all(root).unwrap();
}

/// Cancelling a branch command after it started cannot establish whether the
/// ref changed. The cascade may report its completed lower levels, but it must
/// leave the branch outcome explicitly uncertain.
#[cfg(unix)]
#[test]
fn a_running_cancelled_branch_cascade_does_not_claim_the_branch_survived() {
    use crate::git::{GitMutation, GitServiceHandle, GitServiceState, Repository};

    let root = temporary("uncertain-final-branch");
    fs::create_dir_all(&root).unwrap();
    let root = root.canonicalize().unwrap();
    let target = root.join("removed-linked");
    let repository = Repository::new(&root);
    let mut app = App::new_in_isolated_project(
        &root,
        HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
            String::new(),
        ))))),
    )
    .unwrap();
    app.git.attach(Some(repository));
    let (service, _operations) = GitServiceHandle::recording_for_test();
    app.attach_git_service(service);
    let branch = crate::git::BranchDeletionPlan {
        branch: "feature".to_owned(),
        tip: "0123456789abcdef".to_owned(),
        upstream: None,
        retaining_branches: vec!["main".to_owned()],
        required_authorization: DeletionAuthorization::Typed,
    };
    app.worktree_teardown = Some(WorktreeTeardown {
        plan: WorktreeRemovalPlan {
            path: target.clone(),
            head: Some("0123456789abcdef".to_owned()),
            branch: Some("feature".to_owned()),
            upstream: None,
            detached_retained: false,
            required_authorization: DeletionAuthorization::Typed,
        },
        authorization: DeletionAuthorization::Typed,
        session: None,
        root: target,
        branch: Some(branch.clone()),
        workspace_request_generation: None,
        git_request: None,
        stage: WorktreeTeardownStage::Forgetting,
    });
    app.finish_worktree_teardown();
    let request = app
        .worktree_teardown
        .as_ref()
        .and_then(|teardown| teardown.git_request)
        .unwrap();

    app.apply_git_mutation_result_for_request(
        GitMutation::DeleteBranch {
            plan: Box::new(branch),
            authorization: DeletionAuthorization::Typed,
        },
        Vec::new(),
        None,
        Some(crate::git::GitError::Cancelled {
            command: "git branch -D feature".to_owned(),
        }),
        (Some(request), GitServiceState::CompletedWithUncertainState),
        None,
    );

    assert!(app.worktree_teardown.is_none());
    assert!(app.status_error);
    assert!(app.status.contains("removed worktree"), "{}", app.status);
    assert!(
        app.status.contains("deletion outcome is uncertain"),
        "{}",
        app.status
    );
    assert!(!app.status.contains("was not deleted"), "{}", app.status);
    fs::remove_dir_all(root).unwrap();
}

/// Cancellation and duplicate rejection are outer service errors rather than
/// mutation responses with an inner failure. They still terminate the exact
/// removal that was waiting for that request ID.
#[cfg(unix)]
#[test]
fn an_outer_git_removal_error_abandons_the_matching_teardown() {
    use crate::git::{
        GitServiceEvent, GitServiceHandle, GitServiceState, MemoryGitProvider, Repository,
    };

    let root = temporary("outer-removal-error");
    fs::create_dir_all(&root).unwrap();
    let root = root.canonicalize().unwrap();
    let target = root.join("linked");
    fs::create_dir_all(&target).unwrap();
    let target = target.canonicalize().unwrap();
    let mut ports = HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
        String::new(),
    )))));
    let provider = Rc::new(
        MemoryGitProvider::new(Repository::new(&root)).with_worktrees(vec![test_worktree(
            target.clone(),
            "feature",
            &root,
        )]),
    );
    ports.replace_git(Box::new(Rc::clone(&provider)));
    let mut app = App::new_in_isolated_project(&root, ports).unwrap();
    let (service, operations) = GitServiceHandle::recording_for_test();
    app.attach_git_service(service);
    let plan = crate::git::GitProvider::prepare_worktree_removal(
        provider.as_ref(),
        &Repository::new(&root),
        &target,
    )
    .unwrap();
    app.begin_worktree_teardown(plan, DeletionAuthorization::Typed, None, None);
    let request = app
        .worktree_teardown
        .as_ref()
        .and_then(|teardown| teardown.git_request)
        .expect("the teardown records its removal request");
    let mut removal = None;
    while let Ok(operation) = operations.recv_timeout(std::time::Duration::from_millis(250)) {
        if matches!(
            operation,
            crate::git::GitOperation::Mutate {
                mutation: crate::git::GitMutation::RemoveWorktree { .. },
                ..
            }
        ) {
            removal = Some(operation);
            break;
        }
    }
    let operation = removal.expect("the guarded removal was queued");

    app.apply_git_service_event(GitServiceEvent::Completed {
        id: request,
        operation,
        result: Box::new(Err(crate::git::GitError::Failed {
            command: "remove worktree".to_owned(),
            code: None,
            stderr: "cancelled before the Git operation started".to_owned(),
        })),
        state: GitServiceState::Cancelled,
        coalesced: false,
    });

    assert!(app.worktree_teardown.is_none());
    assert!(app.status_error);
    assert!(app.status.contains("cancelled before"), "{}", app.status);
    fs::remove_dir_all(root).unwrap();
}

/// The session service is attached in either mode, so a standalone editor
/// still finds a persistent session running on a worktree it is removing.
/// Refusing to stop it there would abandon a confirmed removal halfway, for a
/// reason that has nothing to do with the action.
#[cfg(unix)]
#[test]
fn a_standalone_teardown_may_stop_a_session_the_session_commands_would_refuse() {
    let root = temporary("standalone-teardown-stop");
    fs::create_dir_all(&root).unwrap();
    let root = root.canonicalize().unwrap();
    let mut app = App::new_in_isolated_project(
        &root,
        HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
            String::new(),
        ))))),
    )
    .unwrap();
    assert!(!app.persistent_session, "this app is standalone");

    // The typed command still refuses, because it addresses a host this
    // workspace does not have.
    app.execute_command("session-stop").unwrap();
    assert_eq!(app.status, "needs workspace.mode: persistent");

    // The teardown's own stop is not that command, and gets as far as the
    // service. This app has none attached, which is how far it can get here;
    // what matters is that the mode is no longer what turns it back.
    app.status.clear();
    app.status_error = false;
    let _ = app.request_session_stop(root.clone(), false);
    assert_eq!(
        app.status, "session service is unavailable",
        "an internal teardown stop must not be refused for the mode"
    );

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn worktree_typed_confirmation_accepts_pasted_unicode_text() {
    let root = temporary("worktree-pasted-confirmation");
    fs::create_dir_all(&root).unwrap();
    let root = root.canonicalize().unwrap();
    app_with_confirmation(&root, "feat/λ", |app| {
        app.handle_input(InputEvent::Text("feat/λ".to_owned()))
            .unwrap();
        assert_eq!(
            app.git_worktree_removal
                .as_ref()
                .map(|confirmation| confirmation.input.as_str()),
            Some("feat/λ")
        );
        assert_eq!(
            app.git_worktree_removal
                .as_ref()
                .map(|confirmation| confirmation.cursor),
            Some(6)
        );
    });
    fs::remove_dir_all(root).unwrap();
}

fn app_with_confirmation(root: &Path, branch: &str, inspect: impl FnOnce(&mut App)) {
    let mut app = App::new_in_isolated_project(
        root,
        HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
            String::new(),
        ))))),
    )
    .unwrap();
    app.git_worktree_removal = Some(WorktreeRemovalConfirmation {
        session: None,
        plan: WorktreeRemovalPlan {
            path: root.join("linked"),
            head: Some("1".repeat(40)),
            branch: Some(branch.to_owned()),
            upstream: None,
            detached_retained: false,
            required_authorization: DeletionAuthorization::Typed,
        },
        input: String::new(),
        cursor: 0,
    });
    inspect(&mut app);
}

#[test]
fn stale_async_worktree_removal_preflights_never_open_a_confirmation() {
    use crate::{
        app::git_workflows::DeletionPreflight,
        git::{MemoryGitProvider, Repository},
    };

    let root = temporary("worktree-stale-preflight");
    let current = root.join("current");
    let linked = root.join("linked");
    fs::create_dir_all(&current).unwrap();
    fs::create_dir_all(&linked).unwrap();
    let current = current.canonicalize().unwrap();
    let linked = linked.canonicalize().unwrap();
    let repository = Repository::new(&current);
    let rows = vec![
        test_worktree(current.clone(), "main", &root),
        test_worktree(linked.clone(), "feature", &root),
    ];
    let mut ports = HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
        String::new(),
    )))));
    ports.replace_git(Box::new(
        MemoryGitProvider::new(repository.clone()).with_worktrees(rows.clone()),
    ));
    let mut app = App::new_in_isolated_project(&current, ports).unwrap();
    app.open_git_worktrees_result(rows, true);
    let offset = app.active_buffer().line_to_offset(1);
    app.active_mut().replace_selection(Selection::point(offset));
    let source_buffer = app.active().buffer;
    let old_id = GitRequestId::from_raw(51);
    let latest_id = GitRequestId::from_raw(52);
    app.git_state.worktree_removal_request = Some(DeletionPreflight {
        id: latest_id,
        source_buffer,
        interaction_generation: app.next_action_id,
        target: linked.clone(),
    });
    let operation = || GitOperation::PrepareWorktreeRemoval {
        repository: repository.clone(),
        path: linked.clone(),
    };
    let response = || {
        Box::new(Ok(GitResponse::PreparedWorktreeRemoval(
            WorktreeRemovalPlan {
                path: linked.clone(),
                head: Some("1".repeat(40)),
                branch: Some("feature".to_owned()),
                upstream: None,
                detached_retained: false,
                required_authorization: DeletionAuthorization::Enter,
            },
        )))
    };
    app.apply_git_service_event(GitServiceEvent::Completed {
        id: old_id,
        operation: operation(),
        result: response(),
        state: GitServiceState::Completed,
        coalesced: false,
    });
    assert!(app.git_worktree_removal.is_none());
    assert_eq!(
        app.git_state
            .worktree_removal_request
            .as_ref()
            .map(|pending| pending.id),
        Some(latest_id)
    );

    app.open_file(current.join("elsewhere.txt")).unwrap();
    app.apply_git_service_event(GitServiceEvent::Completed {
        id: latest_id,
        operation: operation(),
        result: response(),
        state: GitServiceState::Completed,
        coalesced: false,
    });
    assert!(app.git_worktree_removal.is_none());
    assert!(app.git_state.worktree_removal_request.is_none());
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn stale_initial_session_inspection_never_opens_a_worktree_confirmation() {
    use crate::git::{MemoryGitProvider, Repository};

    let root = temporary("worktree-stale-session-inspection");
    let current = root.join("current");
    let linked = root.join("linked");
    fs::create_dir_all(&current).unwrap();
    fs::create_dir_all(&linked).unwrap();
    let current = current.canonicalize().unwrap();
    let linked = linked.canonicalize().unwrap();
    let rows = vec![
        test_worktree(current.clone(), "main", &root),
        test_worktree(linked.clone(), "feature", &root),
    ];
    let mut ports = HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
        String::new(),
    )))));
    ports.replace_git(Box::new(
        MemoryGitProvider::new(Repository::new(&current)).with_worktrees(rows.clone()),
    ));
    let mut app = App::new_in_isolated_project(&current, ports).unwrap();
    app.open_git_worktrees_result(rows, true);
    let offset = app.active_buffer().line_to_offset(1);
    app.active_mut().replace_selection(Selection::point(offset));
    let source_buffer = app.active().buffer;
    let plan = WorktreeRemovalPlan {
        path: linked.clone(),
        head: Some("1".repeat(40)),
        branch: Some("feature".to_owned()),
        upstream: None,
        detached_retained: false,
        required_authorization: DeletionAuthorization::Enter,
    };
    app.worktree_removal_generation = 11;
    app.pending_worktree_removal = Some(PendingWorktreeRemovalCheck {
        branch: None,
        plan,
        authorization: None,
        origin: Some((source_buffer, app.next_action_id)),
    });

    app.open_file(current.join("elsewhere.txt")).unwrap();
    app.finish_worktree_session_check(11, linked, Ok(None));

    assert!(app.git_worktree_removal.is_none());
    assert!(app.pending_worktree_removal.is_none());
    fs::remove_dir_all(root).unwrap();
}

fn test_worktree(path: PathBuf, branch: &str, root: &Path) -> Worktree {
    Worktree {
        path,
        head: Some("1".repeat(40)),
        branch: Some(format!("refs/heads/{branch}")),
        detached: false,
        bare: false,
        locked: None,
        prunable: None,
        missing: false,
        common_dir: root.join("common"),
    }
}

#[cfg(unix)]
#[test]
fn standalone_refuses_every_session_command_including_the_manager() {
    let root = temporary("standalone-session-list");
    fs::create_dir_all(&root).unwrap();
    let root = root.canonicalize().unwrap();
    let mut app = App::new_in_isolated_project(
        &root,
        HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
            String::new(),
        ))))),
    )
    .unwrap();

    // The manager addresses a host a standalone workspace does not have, so it
    // is inert rather than a list whose every row refuses.
    app.execute_command("sl").unwrap();
    assert_eq!(app.status, "needs workspace.mode: persistent");
    assert!(app.list.is_none(), "standalone opened the session manager");

    app.execute_command(&format!("session-attach {}", root.display()))
        .unwrap();
    assert_eq!(app.status, "needs workspace.mode: persistent");
    assert!(app.take_workspace_switch().is_none());
    app.execute_command("session-stop").unwrap();
    assert_eq!(app.status, "needs workspace.mode: persistent");
    app.execute_command(&format!("session-rename {} other", root.display()))
        .unwrap();
    assert_eq!(app.status, "needs workspace.mode: persistent");

    // The whole namespace reads as unavailable wherever it is discovered,
    // rather than only answering once a key has been pressed.
    let capabilities = app.command_capabilities();
    for name in [
        "session-list",
        "session-attach",
        "session-stop",
        "session-rename",
    ] {
        let spec = crate::command::resolve_command(name).unwrap();
        assert_eq!(
            capabilities.command_availability(spec).reason(),
            Some("needs workspace.mode: persistent"),
            "{name} should be unavailable in standalone mode"
        );
    }
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn a_background_session_refresh_does_not_open_the_manager() {
    let root = temporary("background-session-refresh");
    fs::create_dir_all(&root).unwrap();
    let root = root.canonicalize().unwrap();
    let mut app = App::new_in_isolated_project(
        &root,
        HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
            String::new(),
        ))))),
    )
    .unwrap();
    app.enable_persistent_session();
    app.workspace_generation = 1;

    // Worktree teardown and other session actions refresh the catalog after
    // their own UI has finished. The result updates cached rows, but only an
    // explicit :session-list is allowed to create the manager overlay.
    app.apply_workspace_event(WorkspaceEvent::Refreshed {
        generation: 1,
        result: Ok(Vec::new()),
    });

    assert!(app.list.is_none());
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn session_activity_uses_one_rounded_up_compact_unit() {
    const NOW: u64 = 10 * 24 * 60 * 60;

    assert_eq!(compact_session_elapsed(None, NOW), "-");
    assert_eq!(compact_session_elapsed(Some(NOW), NOW), "0min ago");
    assert_eq!(compact_session_elapsed(Some(NOW - 1), NOW), "1min ago");
    assert_eq!(compact_session_elapsed(Some(NOW - 5 * 60), NOW), "5min ago");
    assert_eq!(
        compact_session_elapsed(Some(NOW - 59 * 60), NOW),
        "59min ago"
    );
    assert_eq!(
        compact_session_elapsed(Some(NOW - (59 * 60 + 1)), NOW),
        "1h ago"
    );
    assert_eq!(
        compact_session_elapsed(Some(NOW - 3 * 60 * 60), NOW),
        "3h ago"
    );
    assert_eq!(
        compact_session_elapsed(Some(NOW - (23 * 60 * 60 + 1)), NOW),
        "1day ago"
    );
    assert_eq!(
        compact_session_elapsed(Some(NOW - 5 * 24 * 60 * 60), NOW),
        "5days ago"
    );
    assert_eq!(compact_session_elapsed(Some(NOW + 60), NOW), "0min ago");
}

#[cfg(unix)]
#[test]
fn session_picker_keeps_filter_and_routes_enter_and_tab_by_workspace_identity() {
    let root = temporary("session-picker");
    let current = root.join("current");
    let stopped = root.join("stopped");
    fs::create_dir_all(&current).unwrap();
    fs::create_dir_all(&stopped).unwrap();
    let current = current.canonicalize().unwrap();
    let stopped = stopped.canonicalize().unwrap();
    let mut app = App::new_in_isolated_project(
        &current,
        HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
            String::new(),
        ))))),
    )
    .unwrap();
    app.enable_persistent_session();
    app.home_directory = Some(root.clone());
    app.workspace_generation = 4;
    open_session_manager_for_refresh(&mut app);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let rows = vec![
        WorkspaceRow {
            id: "aaaaaaaaaaaaaaaa".to_owned(),
            name: Some("current".to_owned()),
            number: None,
            last_active_unix_seconds: None,
            project_root: current.clone(),
            running: true,
            incompatible_protocol: None,
            unsaved_buffers: Some(2),
            pending_wait_requests: Some(0),
            live_terminals: Some(1),
            terminal_sessions: Some(1),
            interactive_attached: Some(true),
            open_buffers: None,
            git: None,
            missing_directory: false,
        },
        WorkspaceRow {
            id: "bbbbbbbbbbbbbbbb".to_owned(),
            name: Some("archive".to_owned()),
            number: None,
            // Thirty seconds inside the five-day ceiling leaves this stable
            // if the test crosses a wall-clock second while rebuilding.
            last_active_unix_seconds: Some(now - 5 * 24 * 60 * 60 + 30),
            project_root: stopped.clone(),
            running: false,
            incompatible_protocol: None,
            unsaved_buffers: Some(0),
            pending_wait_requests: None,
            live_terminals: None,
            terminal_sessions: None,
            interactive_attached: Some(false),
            open_buffers: None,
            git: Some(crate::git::WorkspaceGitFacts {
                branch: Some("main".to_owned()),
                worktree: None,
                remote: None,
            }),
            missing_directory: false,
        },
    ];
    app.apply_workspace_event(WorkspaceEvent::Refreshed {
        generation: 4,
        result: Ok(rows.clone()),
    });
    // Five labelled columns: number, name, branch, directory, activity. Both
    // rows pay for the widest value or heading so the columns stay aligned,
    // and a row with nothing to say in the branch column says `-` rather than
    // going blank and letting the directory slide left.
    let picker = app.list.as_ref().unwrap();
    let header = picker.column_header.as_ref().unwrap();
    assert_eq!(header.label, "No. Name   ");
    assert_eq!(header.detail, "Branch  Path     ");
    assert_eq!(header.trailing_detail, "Last active");
    assert_eq!(picker.items[0].label, "  * current");
    assert_eq!(picker.items[1].label, "    archive");
    assert_eq!(picker.items[0].detail, "-       ~/current");
    assert_eq!(picker.items[0].trailing_detail, "0min ago");
    assert_eq!(picker.items[1].detail, "main    ~/stopped");
    assert_eq!(picker.items[1].trailing_detail, "5days ago");
    assert!(
        picker.items[1]
            .preview()
            .unwrap()
            .contains("Active: 5days ago")
    );
    assert_eq!(picker.primary_action.as_deref(), Some("attach"));
    let overlay = app
        .overlay_snapshots()
        .into_iter()
        .find(|overlay| overlay.title.starts_with("Sessions"))
        .unwrap();
    let header = overlay.column_header.unwrap();
    assert_eq!(header.label, "No. Name   ");
    assert_eq!(header.detail, "Branch  Path     ");
    assert_eq!(header.trailing_detail, "Last active");
    assert!(!app.refresh_workspace_activity_at(now));
    assert!(app.refresh_workspace_activity_at(now + 31));
    assert_eq!(
        app.list.as_ref().unwrap().items[1].trailing_detail,
        "6days ago"
    );
    // Shortening belongs only to presentation: the absolute directory remains
    // a filterable session identity.
    app.list.as_mut().unwrap().filter = crate::git::display_path(&current);
    assert_eq!(app.list.as_ref().unwrap().visible_indices(), vec![0]);
    app.list.as_mut().unwrap().filter = "archive".to_owned();
    app.apply_workspace_event(WorkspaceEvent::Refreshed {
        generation: 4,
        result: Ok(rows),
    });
    assert_eq!(app.list.as_ref().unwrap().filter, "archive");
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert_eq!(
        app.take_workspace_switch().map(|request| request.selector),
        Some(stopped.clone())
    );

    app.workspace_generation = 5;
    open_session_manager_for_refresh(&mut app);
    app.apply_workspace_event(WorkspaceEvent::Refreshed {
        generation: 5,
        result: Ok(vec![WorkspaceRow {
            id: "aaaaaaaaaaaaaaaa".to_owned(),
            name: Some("current".to_owned()),
            number: None,
            last_active_unix_seconds: None,
            project_root: current,
            running: true,
            incompatible_protocol: None,
            unsaved_buffers: Some(2),
            pending_wait_requests: Some(0),
            live_terminals: Some(0),
            terminal_sessions: Some(0),
            interactive_attached: Some(true),
            open_buffers: None,
            git: None,
            missing_directory: false,
        }]),
    });
    key(&mut app, KeyCode::Tab, Modifiers::NONE);
    assert!(app.session_action_menu.is_some());
    assert!(app.overlay_snapshots().iter().any(|overlay| {
        overlay.kind == crate::snapshot::OverlayKind::BufferActions
            && overlay.rows.iter().any(|row| row.label == "Close")
    }));
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn session_picker_omits_counts_a_running_host_answers_with_zero() {
    let root = temporary("session-picker-zero-counts");
    let quiet = root.join("quiet");
    let exited = root.join("exited");
    fs::create_dir_all(&quiet).unwrap();
    fs::create_dir_all(&exited).unwrap();
    let quiet = quiet.canonicalize().unwrap();
    let exited = exited.canonicalize().unwrap();
    let mut app = App::new_in_isolated_project(
        &quiet,
        HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
            String::new(),
        ))))),
    )
    .unwrap();
    app.enable_persistent_session();
    app.workspace_generation = 6;
    open_session_manager_for_refresh(&mut app);
    app.apply_workspace_event(WorkspaceEvent::Refreshed {
        generation: 6,
        result: Ok(vec![
            WorkspaceRow {
                id: "aaaaaaaaaaaaaaaa".to_owned(),
                name: Some("quiet".to_owned()),
                number: None,
                last_active_unix_seconds: None,
                project_root: quiet.clone(),
                running: true,
                incompatible_protocol: None,
                unsaved_buffers: Some(0),
                pending_wait_requests: Some(0),
                live_terminals: Some(0),
                terminal_sessions: Some(0),
                interactive_attached: Some(true),
                open_buffers: None,
                git: None,
                missing_directory: false,
            },
            WorkspaceRow {
                id: "bbbbbbbbbbbbbbbb".to_owned(),
                name: Some("exited".to_owned()),
                number: None,
                last_active_unix_seconds: None,
                project_root: exited.clone(),
                running: true,
                incompatible_protocol: None,
                unsaved_buffers: Some(0),
                pending_wait_requests: Some(0),
                live_terminals: Some(0),
                terminal_sessions: Some(2),
                interactive_attached: Some(true),
                open_buffers: None,
                git: None,
                missing_directory: false,
            },
        ]),
    });
    // A confirmed zero is an answer, so the preview states it rather than
    // leaving the field reading `-` as an unanswered host's would.
    let picker = app.list.as_ref().unwrap();
    let quiet_preview = picker.items[0].preview().unwrap();
    assert!(
        quiet_preview.contains("Status      running"),
        "{quiet_preview}"
    );
    assert!(quiet_preview.contains("Terminals   0"), "{quiet_preview}");
    assert!(quiet_preview.contains("Unsaved     0"), "{quiet_preview}");
    assert!(quiet_preview.contains("Attached    yes"), "{quiet_preview}");
    assert!(
        quiet_preview.contains(&format!("Directory   {}", quiet.display())),
        "{quiet_preview}"
    );
    assert!(quiet_preview.contains("Worktree    no"), "{quiet_preview}");
    // Retained screens whose children have exited are still worth naming, and
    // they are named beside the live count rather than instead of it.
    let exited_preview = picker.items[1].preview().unwrap();
    assert!(
        exited_preview.contains("Terminals   0 (2 exited)"),
        "{exited_preview}"
    );
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn session_picker_marks_a_running_hosts_unanswered_health_as_unavailable() {
    let root = temporary("session-picker-health-unavailable");
    fs::create_dir_all(&root).unwrap();
    let root = root.canonicalize().unwrap();
    let mut app = App::new_in_isolated_project(
        &root,
        HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
            String::new(),
        ))))),
    )
    .unwrap();
    app.enable_persistent_session();
    app.workspace_generation = 7;
    open_session_manager_for_refresh(&mut app);
    app.apply_workspace_event(WorkspaceEvent::Refreshed {
        generation: 7,
        result: Ok(vec![WorkspaceRow {
            id: "aaaaaaaaaaaaaaaa".to_owned(),
            name: Some("unanswered".to_owned()),
            number: None,
            last_active_unix_seconds: None,
            project_root: root.clone(),
            running: true,
            incompatible_protocol: None,
            unsaved_buffers: None,
            pending_wait_requests: None,
            live_terminals: None,
            terminal_sessions: None,
            interactive_attached: None,
            open_buffers: None,
            git: None,
            missing_directory: false,
        }]),
    });

    // A row whose bounded health request went unanswered must not read as a
    // clean session: every host-owned field is unknown, and the status says so
    // rather than the counts quietly reading zero.
    let preview = app.list.as_ref().unwrap().items[0].preview().unwrap();
    assert!(
        preview.contains("Status      running · health unavailable"),
        "{preview}"
    );
    for unknown in [
        "Terminals   -",
        "Buffers     -",
        "Unsaved     -",
        "Attached    -",
    ] {
        assert!(
            preview.contains(unknown),
            "{unknown:?} missing from {preview}"
        );
    }
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn session_picker_states_the_session_as_fields_rather_than_pane_contents() {
    let root = temporary("session-picker-preview");
    fs::create_dir_all(&root).unwrap();
    let root = root.canonicalize().unwrap();
    let mut app = App::new_in_isolated_project(
        &root,
        HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
            String::new(),
        ))))),
    )
    .unwrap();
    app.enable_persistent_session();
    app.workspace_generation = 3;
    open_session_manager_for_refresh(&mut app);
    app.apply_workspace_event(WorkspaceEvent::Refreshed {
        generation: 3,
        result: Ok(vec![WorkspaceRow {
            id: "aaaaaaaaaaaaaaaa".to_owned(),
            name: Some("current".to_owned()),
            number: Some(1),
            last_active_unix_seconds: None,
            project_root: root.clone(),
            running: true,
            incompatible_protocol: None,
            unsaved_buffers: Some(0),
            pending_wait_requests: Some(0),
            live_terminals: Some(2),
            terminal_sessions: Some(3),
            interactive_attached: Some(false),
            open_buffers: Some(9),
            git: Some(crate::git::WorkspaceGitFacts {
                branch: Some("enh/render-space".to_owned()),
                worktree: Some(root.clone()),
                remote: Some("git@example.com:me/runyte.git".to_owned()),
            }),
            missing_directory: false,
        }]),
    });

    let picker = app.list.as_ref().unwrap();
    assert!(picker.has_preview());
    assert_eq!(picker.preview_title(), Some("Session"));
    let preview = picker.selected_preview().unwrap().to_owned();
    for field in [
        "Active: 0min ago",
        "Status      running",
        "Terminals   2 (1 exited)",
        "Buffers     9",
        "Unsaved     0",
        "Attached    no",
        "Branch      enh/render-space",
        &format!("Directory   {}", root.display()),
        "Worktree    yes",
        "Repo        git@example.com:me/runyte.git",
    ] {
        assert!(preview.contains(field), "{field:?} missing from {preview}");
    }
    // The pane count is the one field the lazy control request answers, so it
    // is unknown until that request returns.
    assert!(preview.contains("Panes       -"), "{preview}");

    // The isolated app has no socket service. Model the same result a
    // selected live host returns and verify that applying it fills the pane
    // count without bringing any pane text back into the preview.
    app.workspace_previews.clear();
    app.workspace_preview_generation = 7;
    app.workspace_preview_target = Some(root.clone());
    app.apply_workspace_event(WorkspaceEvent::Previewed {
        generation: 7,
        path: root.clone(),
        result: Ok(SessionPreview {
            layout_panes: 2,
            panes: vec![crate::workspace::SessionPreviewPane {
                active: true,
                title: "[file] src/app.rs".to_owned(),
                kind: crate::workspace::SessionPreviewPaneKind::Buffer {
                    dirty: true,
                    read_only: false,
                },
                start_line: Some(4381),
                lines: vec!["fn rebuild_workspace_picker(&mut self) {".to_owned()],
            }],
            omitted_panes: 0,
            other_resources: vec!["[terminal] cargo test".to_owned()],
            omitted_resources: 0,
        }),
    });

    let preview = app
        .list
        .as_ref()
        .unwrap()
        .selected_preview()
        .unwrap()
        .to_owned();
    assert!(preview.contains("Panes       2"), "{preview}");
    assert!(!preview.contains("src/app.rs"), "{preview}");
    assert!(!preview.contains("rebuild_workspace_picker"), "{preview}");
    assert!(!preview.contains("cargo test"), "{preview}");

    let overlay = app
        .overlay_snapshots()
        .into_iter()
        .find(|overlay| overlay.title.starts_with("Sessions"))
        .unwrap();
    assert_eq!(overlay.layout, crate::snapshot::OverlayLayout::Preview);
    assert!(overlay.show_preview);
    fs::remove_dir_all(root).unwrap();
}

/// Workspace paths are operating-system identities and may contain control
/// characters on Unix. The five-column manager must keep one workspace per
/// visual row just like the branch and worktree buffers do.
#[cfg(unix)]
#[test]
fn session_directory_paths_cannot_manufacture_manager_rows() {
    let root = temporary("session-picker-control-path");
    fs::create_dir_all(&root).unwrap();
    let root = root.canonicalize().unwrap();
    let mut app = App::new_in_isolated_project(
        &root,
        HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
            String::new(),
        ))))),
    )
    .unwrap();
    app.enable_persistent_session();
    app.workspace_generation = 1;
    open_session_manager_for_refresh(&mut app);
    app.apply_workspace_event(WorkspaceEvent::Refreshed {
        generation: 1,
        result: Ok(vec![WorkspaceRow {
            id: "aaaaaaaaaaaaaaaa".to_owned(),
            name: Some("linked".to_owned()),
            number: Some(1),
            last_active_unix_seconds: None,
            project_root: PathBuf::from("/tmp/project\nforged\rroot\tcell"),
            running: false,
            incompatible_protocol: None,
            unsaved_buffers: None,
            pending_wait_requests: None,
            live_terminals: None,
            terminal_sessions: None,
            interactive_attached: None,
            open_buffers: None,
            git: Some(crate::git::WorkspaceGitFacts {
                branch: Some("feature".to_owned()),
                worktree: Some(PathBuf::from("/tmp/linked\nforged\trow")),
                remote: None,
            }),
            missing_directory: false,
        }]),
    });

    let detail = &app.list.as_ref().unwrap().items[0].detail;
    assert_eq!(detail, "feature  /tmp/project\\nforged\\rroot\\tcell");
    assert_eq!(app.list.as_ref().unwrap().items[0].trailing_detail, "-");
    assert!(!detail.contains('\n'));
    assert!(!detail.contains('\t'));
    let preview = app.list.as_ref().unwrap().items[0].preview().unwrap();
    assert!(preview.contains("Directory   /tmp/project\\nforged\\rroot\\tcell"));
    assert!(preview.contains("Worktree    yes"));
    assert!(!preview.contains('\r'));
    assert!(!preview.contains('\t'));
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn the_session_list_marks_stopped_rows_dormant_without_hiding_or_reordering_them() {
    let root = temporary("session-dimming");
    let current = root.join("current");
    let stopped = root.join("stopped");
    fs::create_dir_all(&current).unwrap();
    fs::create_dir_all(&stopped).unwrap();
    let current = current.canonicalize().unwrap();
    let stopped = stopped.canonicalize().unwrap();
    let mut app = App::new_in_isolated_project(
        &current,
        HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
            String::new(),
        ))))),
    )
    .unwrap();
    app.enable_persistent_session();
    app.workspace_generation = 2;
    open_session_manager_for_refresh(&mut app);
    app.apply_workspace_event(WorkspaceEvent::Refreshed {
        generation: 2,
        result: Ok(vec![
            WorkspaceRow {
                id: "aaaaaaaaaaaaaaaa".to_owned(),
                name: Some("current".to_owned()),
                number: None,
                last_active_unix_seconds: None,
                project_root: current,
                running: true,
                incompatible_protocol: None,
                unsaved_buffers: Some(0),
                pending_wait_requests: None,
                live_terminals: None,
                terminal_sessions: None,
                interactive_attached: Some(true),
                open_buffers: None,
                git: None,
                missing_directory: false,
            },
            WorkspaceRow {
                id: "bbbbbbbbbbbbbbbb".to_owned(),
                name: Some("archive".to_owned()),
                number: None,
                last_active_unix_seconds: None,
                project_root: stopped,
                running: false,
                incompatible_protocol: None,
                unsaved_buffers: None,
                pending_wait_requests: None,
                live_terminals: None,
                terminal_sessions: None,
                interactive_attached: None,
                open_buffers: None,
                git: None,
                missing_directory: false,
            },
        ]),
    });

    let overlay = app
        .overlay_snapshots()
        .into_iter()
        .find(|overlay| overlay.title.starts_with("Sessions"))
        .expect("the session list should be open");
    // Both rows are present and in the order the catalog gave them:
    // dimming is the only difference the stopped one carries.
    assert_eq!(overlay.rows.len(), 2);
    assert!(overlay.rows[0].label.contains("current"));
    assert!(!overlay.rows[0].dimmed);
    assert!(overlay.rows[1].label.contains("archive"));
    assert!(overlay.rows[1].dimmed);
    // Dormant is not unavailable: Enter still starts a stopped session.
    assert!(overlay.rows[1].available);
    fs::remove_dir_all(root).unwrap();
}

/// Two numbered sessions and one unnumbered, in a persistent workspace.
#[cfg(unix)]
fn numbered_sessions(label: &str) -> (App, PathBuf, Vec<PathBuf>) {
    let root = temporary(label);
    let roots = ["current", "second", "spare"]
        .iter()
        .map(|name| {
            let path = root.join(name);
            fs::create_dir_all(&path).unwrap();
            path.canonicalize().unwrap()
        })
        .collect::<Vec<_>>();
    let mut app = App::new_in_isolated_project(
        &roots[0],
        HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
            String::new(),
        ))))),
    )
    .unwrap();
    app.enable_persistent_session();
    app.workspace_generation = 3;
    open_session_manager_for_refresh(&mut app);
    let numbers = [Some(1), Some(2), None];
    let names = ["runyte", "runyte-2", "spare"];
    app.apply_workspace_event(WorkspaceEvent::Refreshed {
        generation: 3,
        result: Ok(roots
            .iter()
            .zip(numbers)
            .zip(names)
            .map(|((project_root, number), name)| WorkspaceRow {
                id: format!("{name}00000000000000"),
                name: Some(name.to_owned()),
                number,
                last_active_unix_seconds: None,
                project_root: project_root.clone(),
                running: true,
                incompatible_protocol: None,
                unsaved_buffers: Some(0),
                pending_wait_requests: None,
                live_terminals: None,
                terminal_sessions: None,
                interactive_attached: Some(false),
                open_buffers: None,
                git: None,
                missing_directory: false,
            })
            .collect()),
    });
    (app, root, roots)
}

#[cfg(unix)]
#[test]
fn a_digit_attaches_to_a_numbered_session_from_the_manager() {
    let (mut app, root, roots) = numbered_sessions("session-number-attach");

    press(&mut app, '2');
    assert_eq!(
        app.take_workspace_switch().map(|request| request.selector),
        Some(roots[1].clone()),
        "the digit reaches the session holding that number, not the second row"
    );
    assert!(app.list.is_none(), "attaching closes the manager");
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn space_closes_the_session_manager_instead_of_filtering() {
    let (mut app, root, _roots) = numbered_sessions("session-space-close");

    press(&mut app, ' ');

    assert!(app.list.is_none(), "Space dismisses the manager overlay");
    assert!(app.take_workspace_switch().is_none());
    fs::remove_dir_all(root).unwrap();
}

/// The reason a digit is a shortcut only while nothing has been typed.
/// Runyte's own default names and this repository's own worktree path both
/// contain digits, so filtering for them has to keep working.
#[cfg(unix)]
#[test]
fn a_digit_typed_into_a_filter_is_filter_text_rather_than_a_shortcut() {
    let (mut app, root, _roots) = numbered_sessions("session-number-filter");

    for character in "runyte-".chars() {
        press(&mut app, character);
    }
    press(&mut app, '2');
    assert!(
        app.take_workspace_switch().is_none(),
        "a digit after other text must not attach"
    );
    let list = app.list.as_ref().unwrap();
    assert_eq!(list.filter, "runyte-2");
    assert_eq!(
        list.visible_indices().len(),
        1,
        "the filter still narrows to the digit-bearing name"
    );
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn clearing_the_filter_arms_the_digit_shortcut_again() {
    let (mut app, root, roots) = numbered_sessions("session-number-rearm");

    press(&mut app, 'r');
    press(&mut app, '1');
    assert!(app.take_workspace_switch().is_none());
    assert_eq!(app.list.as_ref().unwrap().filter, "r1");

    key(&mut app, KeyCode::Delete, Modifiers::NONE);
    assert!(app.list.as_ref().unwrap().filter.is_empty());
    press(&mut app, '1');
    assert_eq!(
        app.take_workspace_switch().map(|request| request.selector),
        Some(roots[0].clone()),
        "an emptied filter is the state the shortcut is armed in"
    );
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn a_digit_no_session_holds_reports_that_rather_than_attaching() {
    let (mut app, root, _roots) = numbered_sessions("session-number-missing");

    press(&mut app, '7');
    assert!(app.take_workspace_switch().is_none());
    assert!(
        app.status.contains("no session is numbered 7"),
        "unexpected message: {}",
        app.status
    );
    assert!(app.status_error);
    assert!(app.list.is_some(), "a wrong digit leaves the manager open");
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn the_manager_renumber_action_opens_an_empty_prompt() {
    let (mut app, root, roots) = numbered_sessions("session-number-prompt");

    app.list.as_mut().unwrap().selected = 1;
    key(&mut app, KeyCode::Tab, Modifiers::NONE);
    app.session_action_menu.as_mut().unwrap().selected = 2;
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert_eq!(app.prompt_kind, PromptKind::SessionNumber);
    assert!(
        app.command.is_empty(),
        "renumber starts ready for one digit"
    );
    assert_eq!(
        app.session_number_target.as_deref(),
        Some(roots[1].as_path())
    );
    assert!(app.list.is_none(), "the scalar prompt owns its input");
    press(&mut app, '3');
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert!(app.session_number_target.is_none());
    assert_eq!(app.mode, Mode::Normal);
    assert!(
        app.list.is_some(),
        "accepting Renumber returns to the session manager"
    );
    assert_eq!(
        app.list.as_ref().unwrap().selected,
        1,
        "the session being renumbered remains selected"
    );

    key(&mut app, KeyCode::Tab, Modifiers::NONE);
    app.session_action_menu.as_mut().unwrap().selected = 2;
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    key(&mut app, KeyCode::Escape, Modifiers::NONE);
    assert!(app.session_number_target.is_none());
    assert!(
        app.list.is_some(),
        "cancelling Renumber also returns to the session manager"
    );
    assert_eq!(app.list.as_ref().unwrap().selected, 1);
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn the_session_manager_initially_selects_the_current_session() {
    let (mut app, root, roots) = numbered_sessions("session-current-selection");
    app.project_root = roots[1].clone();
    open_session_manager_for_refresh(&mut app);

    app.rebuild_workspace_picker();

    assert_eq!(
        app.list.as_ref().unwrap().selected,
        1,
        "the initial row follows the current workspace instead of the first row"
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_session_number_answer_accepts_one_to_nine_and_an_empty_clearing() {
    assert_eq!(parse_session_number("3"), Ok(Some(3)));
    assert_eq!(parse_session_number(" 9 "), Ok(Some(9)));
    assert_eq!(
        parse_session_number(""),
        Ok(None),
        "empty clears the number"
    );
    assert_eq!(parse_session_number("   "), Ok(None));
    assert!(parse_session_number("0").is_err());
    assert!(parse_session_number("10").is_err());
    assert!(parse_session_number("one").is_err());
}

#[cfg(unix)]
#[test]
fn workspace_actions_match_the_selected_session_state() {
    let root = temporary("workspace-actions");
    let current = root.join("current");
    let stopped = root.join("stopped");
    fs::create_dir_all(&current).unwrap();
    fs::create_dir_all(&stopped).unwrap();
    let current = current.canonicalize().unwrap();
    let stopped = stopped.canonicalize().unwrap();
    let mut app = App::new_in_isolated_project(
        &current,
        HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
            String::new(),
        ))))),
    )
    .unwrap();
    app.enable_persistent_session();
    app.workspace_generation = 9;
    open_session_manager_for_refresh(&mut app);
    app.apply_workspace_event(WorkspaceEvent::Refreshed {
        generation: 9,
        result: Ok(vec![
            WorkspaceRow {
                id: "aaaaaaaaaaaaaaaa".to_owned(),
                name: Some("current".to_owned()),
                number: None,
                last_active_unix_seconds: None,
                project_root: current,
                running: true,
                incompatible_protocol: None,
                unsaved_buffers: Some(0),
                pending_wait_requests: None,
                live_terminals: None,
                terminal_sessions: None,
                interactive_attached: Some(true),
                open_buffers: None,
                git: None,
                missing_directory: false,
            },
            WorkspaceRow {
                id: "bbbbbbbbbbbbbbbb".to_owned(),
                name: Some("archive".to_owned()),
                number: None,
                last_active_unix_seconds: None,
                project_root: stopped.clone(),
                running: false,
                incompatible_protocol: None,
                unsaved_buffers: Some(0),
                pending_wait_requests: None,
                live_terminals: None,
                terminal_sessions: None,
                interactive_attached: Some(false),
                open_buffers: None,
                git: None,
                missing_directory: false,
            },
        ]),
    });

    // A running row can be stopped but not forgotten.
    key(&mut app, KeyCode::Tab, Modifiers::NONE);
    assert_eq!(
        app.session_action_menu.as_ref().unwrap().actions,
        vec![
            SessionAction::Open,
            SessionAction::Rename,
            SessionAction::Number,
            SessionAction::Close,
            SessionAction::ForceClose,
        ]
    );
    key(&mut app, KeyCode::Escape, Modifiers::NONE);

    // A stopped row can be forgotten but neither closed nor force closed, and
    // holds no digit to renumber.
    key(&mut app, KeyCode::Down, Modifiers::NONE);
    key(&mut app, KeyCode::Tab, Modifiers::NONE);
    assert_eq!(
        app.session_action_menu.as_ref().unwrap().actions,
        vec![
            SessionAction::Open,
            SessionAction::Rename,
            SessionAction::Forget,
        ]
    );
    let labels = app
        .overlay_snapshots()
        .iter()
        .find(|overlay| overlay.kind == crate::snapshot::OverlayKind::BufferActions)
        .map(|overlay| {
            overlay
                .rows
                .iter()
                .map(|row| row.label.clone())
                .collect::<Vec<_>>()
        })
        .unwrap();
    assert_eq!(labels, vec!["Open", "Rename", "Forget"]);

    // No session service is attached in an isolated project, so the
    // request cannot be served; what matters here is that Forget asks to
    // forget rather than to stop, and that the picker stays open.
    app.session_action_menu.as_mut().unwrap().selected = 2;
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert!(app.take_workspace_switch().is_none());
    assert!(app.list.is_some());

    app.workspace_generation = 10;
    app.apply_workspace_event(WorkspaceEvent::Forgotten {
        generation: 10,
        path: stopped.clone(),
        result: Ok(true),
    });
    assert!(app.session_action_menu.is_none());

    key(&mut app, KeyCode::Tab, Modifiers::NONE);
    app.session_action_menu.as_mut().unwrap().selected = 1;
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert_eq!(app.prompt_kind, PromptKind::SessionRename);
    assert_eq!(app.command, "archive");
    assert_eq!(
        app.session_rename_target.as_deref(),
        Some(stopped.as_path())
    );
    key(&mut app, KeyCode::Escape, Modifiers::NONE);
    assert!(app.session_rename_target.is_none());
    fs::remove_dir_all(root).unwrap();
}

/// The menu's action list is fixed when it opens, but a refresh can change
/// the row underneath it, so Enter re-reads the live state before acting.
/// Force close additionally takes two presses.
#[cfg(unix)]
#[test]
fn session_actions_confirm_force_close_and_recheck_state_at_enter() {
    let root = temporary("workspace-action-state");
    let current = root.join("current");
    let stopped = root.join("stopped");
    fs::create_dir_all(&current).unwrap();
    fs::create_dir_all(&stopped).unwrap();
    let current = current.canonicalize().unwrap();
    let stopped = stopped.canonicalize().unwrap();
    let mut app = App::new_in_isolated_project(
        &current,
        HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
            String::new(),
        ))))),
    )
    .unwrap();
    app.enable_persistent_session();

    // An isolated project has no session service, so a request that gets
    // past the state guards fails with that error rather than being
    // served. That is what separates "the guard refused" from "the guard
    // let it through".
    let rows = |running: bool| {
        vec![
            WorkspaceRow {
                id: "aaaaaaaaaaaaaaaa".to_owned(),
                name: Some("current".to_owned()),
                number: None,
                last_active_unix_seconds: None,
                project_root: current.clone(),
                running,
                incompatible_protocol: None,
                unsaved_buffers: Some(0),
                pending_wait_requests: None,
                live_terminals: None,
                terminal_sessions: None,
                interactive_attached: Some(true),
                open_buffers: None,
                git: None,
                missing_directory: false,
            },
            WorkspaceRow {
                id: "bbbbbbbbbbbbbbbb".to_owned(),
                name: Some("archive".to_owned()),
                number: None,
                last_active_unix_seconds: None,
                project_root: stopped.clone(),
                running: !running,
                incompatible_protocol: None,
                unsaved_buffers: Some(0),
                pending_wait_requests: None,
                live_terminals: None,
                terminal_sessions: None,
                interactive_attached: Some(false),
                open_buffers: None,
                git: None,
                missing_directory: false,
            },
        ]
    };
    let refresh = |app: &mut App, running: bool| {
        app.workspace_generation = app.workspace_generation.wrapping_add(1).max(1);
        let generation = app.workspace_generation;
        app.apply_workspace_event(WorkspaceEvent::Refreshed {
            generation,
            result: Ok(rows(running)),
        });
    };

    open_session_manager_for_refresh(&mut app);
    refresh(&mut app, true);
    key(&mut app, KeyCode::Tab, Modifiers::NONE);
    assert_eq!(
        app.session_action_menu.as_ref().unwrap().actions[4],
        SessionAction::ForceClose
    );

    // The first Enter only arms the confirmation and keeps the menu open.
    app.session_action_menu.as_mut().unwrap().selected = 4;
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert!(app.session_action_menu.as_ref().unwrap().force_armed);
    assert!(!app.status_error);
    assert!(
        app.status.contains("press Enter again to confirm"),
        "{}",
        app.status
    );

    // Moving in either direction disarms it, so the confirmation is never
    // inherited by whatever action the selection lands on.
    key(&mut app, KeyCode::Up, Modifiers::NONE);
    assert!(!app.session_action_menu.as_ref().unwrap().force_armed);
    key(&mut app, KeyCode::Down, Modifiers::NONE);
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert!(app.session_action_menu.as_ref().unwrap().force_armed);
    key(&mut app, KeyCode::Down, Modifiers::NONE);
    assert!(!app.session_action_menu.as_ref().unwrap().force_armed);

    // Arming again and confirming reaches the request itself.
    key(&mut app, KeyCode::Up, Modifiers::NONE);
    assert_eq!(
        app.session_action_menu.as_ref().unwrap().selected_action(),
        Some(SessionAction::ForceClose)
    );
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert!(app.session_action_menu.as_ref().unwrap().force_armed);
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert!(app.status_error);
    assert_eq!(app.status, "session service is unavailable");

    // A refresh can stop the row while its menu is open. The menu still
    // lists Close, but Enter reads the row as it is now and refuses.
    key(&mut app, KeyCode::Escape, Modifiers::NONE);
    key(&mut app, KeyCode::Tab, Modifiers::NONE);
    assert_eq!(
        app.session_action_menu.as_ref().unwrap().actions[3],
        SessionAction::Close
    );
    app.session_action_menu.as_mut().unwrap().selected = 3;
    refresh(&mut app, false);
    assert!(app.session_action_menu.is_some());
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert_eq!(app.status, "this session is already stopped");

    // Renumber is guarded the same way, so a row cannot regain a number after
    // it stops while its old running-row menu remains open.
    app.session_action_menu.as_mut().unwrap().selected = 2;
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert_eq!(app.status, "this session is already stopped");
    assert_ne!(app.prompt_kind, PromptKind::SessionNumber);
    assert!(app.session_number_target.is_none());

    // Force close is refused the same way, and never arms for a row that
    // is no longer running.
    app.session_action_menu.as_mut().unwrap().selected = 4;
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert!(!app.session_action_menu.as_ref().unwrap().force_armed);
    assert_eq!(app.status, "this session is already stopped");

    // The mirror image: a stopped row's menu offers Forget, but a refresh
    // starts the session before Enter lands.
    key(&mut app, KeyCode::Escape, Modifiers::NONE);
    key(&mut app, KeyCode::Tab, Modifiers::NONE);
    assert_eq!(
        app.session_action_menu.as_ref().unwrap().actions[2],
        SessionAction::Forget
    );
    app.session_action_menu.as_mut().unwrap().selected = 2;
    refresh(&mut app, true);
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert_eq!(app.status, "stop this session before forgetting it");

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn workspace_switch_requests_are_platform_guarded_persistent_and_preserve_dirty_hosts() {
    let root = temporary("workspace-switch-scratch");
    let current = root.join("current");
    let destination = root.join("destination");
    fs::create_dir_all(&current).unwrap();
    fs::create_dir_all(&destination).unwrap();
    let current = current.canonicalize().unwrap();
    let destination = destination.canonicalize().unwrap();
    let file = current.join("note.txt");
    fs::write(&file, "saved\n").unwrap();
    let mut app = App::new_in_isolated_project(
        &current,
        HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
            String::new(),
        ))))),
    )
    .unwrap();

    // Standalone mode never leaves its in-process host, regardless of
    // whether the only edit is in the scratchpad.
    seed(&mut app, "a note to self");
    assert!(app.buffers[0].dirty);
    assert!(!app.request_workspace_switch_for_platform(destination.clone(), false));
    assert!(app.take_workspace_switch().is_none());
    assert_eq!(
        app.status,
        crate::service_health::PERSISTENT_SESSION_UNSUPPORTED_REASON
    );
    assert!(!app.request_workspace_switch_for_platform(destination.clone(), true));
    assert!(app.take_workspace_switch().is_none());
    assert!(app.status.contains("workspace.mode: persistent"));

    app.open_file(file).unwrap();
    let buffer = app.active().buffer;
    app.buffers[buffer].apply(&Transaction::insert(0, "unsaved "));
    app.enable_persistent_session();
    assert!(app.request_workspace_switch_for_platform(destination.clone(), true));
    assert_eq!(
        app.take_workspace_switch().map(|request| request.selector),
        Some(destination)
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn worktree_shift_d_confirms_cancels_and_removes_only_the_typed_path() {
    use crate::git::{MemoryGitProvider, Repository};

    let root = temporary("general-worktree-remove");
    let current = root.join("current");
    let linked = root.join("linked");
    fs::create_dir_all(&current).unwrap();
    fs::create_dir_all(&linked).unwrap();
    let current = current.canonicalize().unwrap();
    let linked = linked.canonicalize().unwrap();
    let worktree = |path: PathBuf, branch: &str| Worktree {
        path,
        head: Some("0123456789abcdef".to_owned()),
        branch: Some(format!("refs/heads/{branch}")),
        detached: false,
        bare: false,
        locked: None,
        prunable: None,
        missing: false,
        common_dir: root.join("common"),
    };
    let rows = vec![
        worktree(current.clone(), "main"),
        worktree(linked.clone(), "feature"),
    ];
    let provider = Rc::new(
        MemoryGitProvider::new(Repository::new(&current))
            .with_branches(&["feature", "main"], "main")
            .with_branch_checkout("main", current.clone())
            .with_branch_checkout("feature", linked.clone())
            .with_worktrees(rows.clone()),
    );
    let mut ports = HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
        String::new(),
    )))));
    ports.replace_git(Box::new(Rc::clone(&provider)));
    let mut app = App::new_in_isolated_project(&current, ports).unwrap();
    app.open_git_worktrees_result(rows, true);
    let offset = app.active_buffer().line_to_offset(1);
    app.active_mut().replace_selection(Selection::point(offset));

    context_action(&mut app, 'D');
    assert_eq!(
        app.git_worktree_removal
            .as_ref()
            .map(|confirmation| confirmation.plan.path.as_path()),
        Some(linked.as_path())
    );
    assert!(app.status.contains(&crate::git::display_path(&linked)));
    assert!(app.status.contains("Branch feature will remain"));
    key(&mut app, KeyCode::Escape, Modifiers::NONE);
    assert!(provider.removed_worktrees().is_empty());
    assert!(linked.exists());

    context_action(&mut app, 'D');
    app.status("unrelated service feedback");
    let overlay = confirmation_snapshot(&app);
    assert_eq!(overlay.title, "Remove worktree");
    assert_eq!(overlay.actions[0].label, "remove worktree");
    let question = overlay
        .message
        .expect("worktree removal keeps its own confirmation question");
    assert!(question.contains(&crate::git::display_path(&linked)));
    assert!(question.contains("Branch feature will remain"));
    assert!(!question.contains("unrelated service feedback"));
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert_eq!(provider.removed_worktrees(), vec![linked.clone()]);
    assert!(
        provider
            .branches(&Repository::new(&current))
            .unwrap()
            .iter()
            .any(|branch| branch.name == "feature")
    );
    assert!(
        !app.active_buffer()
            .to_string()
            .contains(&crate::git::display_path(&linked))
    );
    assert!(app.status.contains("no branch was deleted"));

    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn worktree_control_path_is_one_safe_row_and_confirmation_with_typed_identity() {
    use std::{ffi::OsString, os::unix::ffi::OsStringExt};

    use crate::git::{MemoryGitProvider, Repository};

    let root = temporary("general-worktree-remove-control-path");
    let current = root.join("current");
    fs::create_dir_all(&current).unwrap();
    let current = current.canonicalize().unwrap();
    // macOS rejects non-UTF-8 path components with EILSEQ. It still covers
    // every control-character escape here; Linux additionally covers the
    // lossy rendering of an otherwise valid raw filename byte.
    let linked_name = if cfg!(target_os = "macos") {
        b"linked-\n-\t-\\-\x1b".to_vec()
    } else {
        b"linked-\n-\t-\\-\x1b-\xff".to_vec()
    };
    let linked = root.join(OsString::from_vec(linked_name));
    fs::create_dir_all(&linked).unwrap();
    let linked = linked.canonicalize().unwrap();
    let row = |path: PathBuf, branch: &str| Worktree {
        path,
        head: Some("0123456789abcdef".to_owned()),
        branch: Some(format!("refs/heads/{branch}")),
        detached: false,
        bare: false,
        locked: None,
        prunable: None,
        missing: false,
        common_dir: root.join("common"),
    };
    let rows = vec![row(current.clone(), "main"), row(linked.clone(), "feature")];
    let provider =
        Rc::new(MemoryGitProvider::new(Repository::new(&current)).with_worktrees(rows.clone()));
    let mut ports = HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
        String::new(),
    )))));
    ports.replace_git(Box::new(Rc::clone(&provider)));
    let mut app = App::new_in_isolated_project(&current, ports).unwrap();
    app.open_git_worktrees_result(rows, true);
    let text = app.active_buffer().to_string();
    let display = crate::git::display_path(&linked);
    #[cfg(target_os = "macos")]
    let expected_suffix = "linked-\\n-\\t-\\-\\u{1b}";
    #[cfg(not(target_os = "macos"))]
    let expected_suffix = "linked-\\n-\\t-\\-\\u{1b}-�";
    assert!(display.ends_with(expected_suffix), "{display:?}");
    assert_eq!(text.lines().count(), 2, "{text:?}");
    assert!(text.contains("\\n"), "{text:?}");
    assert!(text.contains("\\t"), "{text:?}");
    assert!(text.contains("\\u{1b}"), "{text:?}");
    let offset = app.active_buffer().line_to_offset(1);
    app.active_mut().replace_selection(Selection::point(offset));

    context_action(&mut app, 'D');
    let confirmation = app.git_worktree_removal.as_ref().unwrap();
    assert_eq!(confirmation.plan.path, linked);
    let question = confirmation.message();
    assert!(
        question
            .chars()
            .all(|character| character == '\n' || !character.is_control())
    );
    assert_eq!(question.lines().count(), 4);
    assert!(question.contains("\\n"));
    assert!(question.contains("\\t"));
    assert!(question.contains("\\u{1b}"));
    key(&mut app, KeyCode::Escape, Modifiers::NONE);
    assert!(app.status.chars().all(|character| !character.is_control()));
    assert!(app.status.contains("\\n"));
    assert!(provider.removed_worktrees().is_empty());

    context_action(&mut app, 'D');
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert_eq!(provider.removed_worktrees(), vec![linked]);

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn worktree_removal_refuses_current_locked_bare_and_unavailable_rows_before_confirmation() {
    let root = temporary("general-worktree-remove-refusals");
    let current = root.join("current");
    let linked = root.join("linked");
    fs::create_dir_all(&current).unwrap();
    fs::create_dir_all(&linked).unwrap();
    let current = current.canonicalize().unwrap();
    let linked = linked.canonicalize().unwrap();
    let row = |path: PathBuf| Worktree {
        path,
        head: Some("0123456789abcdef".to_owned()),
        branch: Some("refs/heads/feature".to_owned()),
        detached: false,
        bare: false,
        locked: None,
        prunable: None,
        missing: false,
        common_dir: root.join("common"),
    };
    let mut locked = row(linked.clone());
    locked.locked = Some("maintenance".to_owned());
    let mut bare = row(root.join("bare"));
    bare.bare = true;
    let mut missing = row(root.join("missing"));
    missing.missing = true;
    let mut app = App::new_in_isolated_project(
        &current,
        HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
            String::new(),
        ))))),
    )
    .unwrap();
    app.open_git_worktrees_result(vec![row(current), locked, bare, missing], true);
    for (line, expected) in [
        (0, "workspace is using"),
        (1, "locked"),
        (2, "bare"),
        (3, "unavailable"),
    ] {
        let offset = app.active_buffer().line_to_offset(line);
        app.active_mut().replace_selection(Selection::point(offset));
        app.remove_selected_worktree();
        assert!(
            app.status_error && app.status.contains(expected),
            "{}",
            app.status
        );
        assert!(app.git_worktree_removal.is_none());
    }
    fs::remove_dir_all(root).unwrap();
}
