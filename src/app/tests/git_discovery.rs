// SPDX-License-Identifier: MPL-2.0

use super::*;
use crate::git::GitError;
use std::sync::mpsc::Receiver;

fn discovery_app() -> (App, Receiver<GitOperation>) {
    let mut app = App::new(Config::default(), None).unwrap();
    let (service, operations) = GitServiceHandle::recording_for_test();
    app.attach_git_service(service);
    (app, operations)
}

fn next_discovery(operations: &Receiver<GitOperation>) -> GitOperation {
    let operation = operations.recv_timeout(Duration::from_secs(2)).unwrap();
    assert!(matches!(operation, GitOperation::Discover { .. }));
    operation
}

fn answer(
    app: &mut App,
    id: u64,
    operation: GitOperation,
    result: crate::git::Result<GitResponse>,
) {
    let state = if result.is_ok() {
        GitServiceState::Completed
    } else {
        GitServiceState::Failed
    };
    app.apply_git_service_event(GitServiceEvent::Completed {
        id: GitRequestId::from_raw(id),
        operation,
        result: Box::new(result),
        state,
        coalesced: false,
    });
}

fn git_health(app: &App) -> ServiceHealthEntry {
    app.service_health_snapshot()
        .entries
        .into_iter()
        .find(|entry| entry.service == "git")
        .unwrap()
}

fn refresh_available(app: &App) -> bool {
    app.matching_commands()
        .iter()
        .find(|row| row.name == "git-refresh")
        .unwrap()
        .availability
        .is_available()
}

fn launch_failure() -> GitError {
    GitError::Unavailable {
        detail: "cannot start controlled Git child".to_owned(),
    }
}

#[test]
fn discovery_failures_require_one_explicit_retry_and_keep_diagnostics_until_answered() {
    // The controlled boundary distinguishes a provider that could not launch
    // from a child returning a Git exit code and an indeterminate signal exit.
    // Permission/configuration failures follow the same explicit-only policy.
    for failure in [
        launch_failure(),
        GitError::Failed {
            command: "git rev-parse".to_owned(),
            code: Some(128),
            signal: None,
            stderr: "invalid repository configuration".to_owned(),
        },
        GitError::Failed {
            command: "git rev-parse".to_owned(),
            code: None,
            signal: Some(6),
            stderr: String::new(),
        },
        GitError::Io {
            action: "inspect",
            path: PathBuf::from("/project/.git"),
            detail: "permission denied".to_owned(),
        },
        GitError::Cancelled {
            command: "git rev-parse".to_owned(),
        },
    ] {
        let (mut app, operations) = discovery_app();
        let initial = next_discovery(&operations);
        assert!(!refresh_available(&app));
        assert_eq!(git_health(&app).state, ServiceState::Idle);
        answer(&mut app, 1, initial, Err(failure.clone()));
        assert_eq!(app.git_state.discovery_error(), Some(&failure));
        assert!(refresh_available(&app));
        assert!(!app.command_capabilities().git_project.is_available());
        assert_eq!(git_health(&app).state, ServiceState::Degraded);
        assert!(git_health(&app).detail.contains(&failure.to_string()));
        assert!(app.git_summary().unwrap().contains(":git-refresh"));
        let notification = app.notifications.entries()[0].clone();
        assert!(notification.body.contains(&failure.to_string()));
        assert!(notification.body.contains(":git-refresh"));

        // Neither ordinary input nor the host's maintenance entry points
        // submit discovery. No backoff deadline exists to expire later.
        press(&mut app, 'i');
        press(&mut app, 'x');
        key(&mut app, KeyCode::Escape, Modifiers::NONE);
        app.refresh_git_status();
        assert!(
            app.request_automatic_git_refresh(RefreshSpec::default())
                .is_none()
        );
        assert!(!app.retry_pending_git_reconciliation(Instant::now() + Duration::from_secs(3600)));
        assert!(operations.try_recv().is_err());

        if matches!(failure, GitError::Unavailable { .. }) {
            for character in [' ', 'g', 'r'] {
                press(&mut app, character);
            }
        } else {
            app.execute_command("git-refresh").unwrap();
        }
        let retry = next_discovery(&operations);
        assert!(!refresh_available(&app));
        assert!(git_health(&app).detail.contains("retry is in progress"));
        assert!(git_health(&app).detail.contains(&failure.to_string()));
        assert_eq!(app.git_state.discovery_error(), Some(&failure));
        assert_eq!(
            app.git_summary().as_deref(),
            Some("git · retrying discovery")
        );
        app.execute_command("git-refresh").unwrap();
        app.refresh_git(); // The workflow also guards callers below the registry.
        assert!(operations.try_recv().is_err());
        press(&mut app, 'i');
        press(&mut app, 'y');
        key(&mut app, KeyCode::Escape, Modifiers::NONE);
        assert!(app.active_buffer().to_string().contains('y'));

        let repository = Repository::new(&app.project_root);
        answer(
            &mut app,
            2,
            retry,
            Ok(GitResponse::Discovered(Some(repository.clone()))),
        );
        assert_eq!(app.git.repository(), Some(&repository));
        assert!(app.git_state.discovery_error().is_none());
        assert!(refresh_available(&app));
        assert!(app.command_capabilities().git_project.is_available());
        assert_eq!(git_health(&app).state, ServiceState::Ready);
        assert!(app.notifications.entries().contains(&notification));
        assert!(
            matches!(operations.recv_timeout(Duration::from_secs(2)).unwrap(),
            GitOperation::Refresh { repository: found, .. } if found == repository)
        );
    }
}

#[test]
fn repeated_discovery_failure_replaces_last_error_and_absence_ends_retry() {
    let (mut app, operations) = discovery_app();
    answer(
        &mut app,
        1,
        next_discovery(&operations),
        Err(launch_failure()),
    );
    app.execute_command("git-refresh").unwrap();
    let refusal = GitError::Failed {
        command: "git rev-parse".to_owned(),
        code: Some(128),
        signal: None,
        stderr: "bad configuration".to_owned(),
    };
    answer(
        &mut app,
        2,
        next_discovery(&operations),
        Err(refusal.clone()),
    );
    assert_eq!(app.git_state.discovery_error(), Some(&refusal));
    assert_eq!(app.notifications.entries().len(), 2);
    assert!(refresh_available(&app));
    assert!(operations.try_recv().is_err());

    app.execute_command("git-refresh").unwrap();
    answer(
        &mut app,
        3,
        next_discovery(&operations),
        Ok(GitResponse::Discovered(None)),
    );
    assert!(app.git_state.discovery_error().is_none());
    assert!(!refresh_available(&app));
    assert_eq!(git_health(&app).state, ServiceState::Idle);
    assert_eq!(
        git_health(&app).detail,
        "this project is not in a Git repository"
    );
    assert!(app.git_summary().is_none());
    assert_eq!(app.notifications.entries().len(), 2);
    app.execute_command("git-refresh").unwrap();
    assert!(operations.try_recv().is_err());
}

#[test]
fn rejected_discovery_submission_is_retryable_and_keeps_the_original_failure() {
    let mut app = App::new(Config::default(), None).unwrap();
    assert!(!refresh_available(&app));
    assert_eq!(git_health(&app).state, ServiceState::Unavailable);
    let (service, paused) = GitServiceHandle::saturated_for_test();
    app.attach_git_service(service);
    assert!(
        refresh_available(&app),
        "queue rejection must not leave discovery pending"
    );
    let original = app.git_state.discovery_error().unwrap().clone();
    drop(paused); // Later submission fails differently: the service has stopped.
    app.execute_command("git-refresh").unwrap();
    assert!(refresh_available(&app));
    assert_eq!(app.git_state.discovery_error(), Some(&original));
    assert!(app.status.contains("Git service has stopped"));
    assert!(!app.status.contains("not in a Git repository"));
}

#[test]
fn discovery_can_be_submitted_after_queue_capacity_returns() {
    let mut app = App::new(Config::default(), None).unwrap();
    let (service, paused) = GitServiceHandle::saturated_for_test();
    app.attach_git_service(service);
    let original = app.git_state.discovery_error().unwrap().clone();
    paused.next_operation();
    app.execute_command("git-refresh").unwrap();
    assert!(!app.git_state.discovery_complete());
    assert_eq!(app.git_state.discovery_error(), Some(&original));
    assert!(!refresh_available(&app));
    assert!(app.status.contains("retrying Git repository discovery"));
}

#[test]
fn synchronous_test_provider_can_recover_failed_discovery() {
    use crate::git::MemoryGitProvider;

    let mut app = App::new(Config::default(), None).unwrap();
    let repository = Repository::new(&app.project_root);
    app.ports.replace_git(Box::new(
        MemoryGitProvider::new(repository.clone()).failing(),
    ));
    app.attach_repository();
    let original = app.git_state.discovery_error().unwrap().clone();
    app.execute_command("git-refresh").unwrap();
    assert_eq!(app.git_state.discovery_error(), Some(&original));
    assert!(app.status_error);
    app.ports
        .replace_git(Box::new(MemoryGitProvider::new(repository.clone())));
    app.execute_command("git-refresh").unwrap();
    assert!(app.git_state.discovery_error().is_none());
    assert_eq!(app.git.repository(), Some(&repository));
    assert!(refresh_available(&app));
    assert!(!app.status_error);
}

#[test]
fn persistent_frontend_reattachment_retains_failed_and_pending_discovery() {
    let (mut app, operations) = discovery_app();
    app.persistent_session = true;
    answer(
        &mut app,
        1,
        next_discovery(&operations),
        Err(launch_failure()),
    );
    for retry_pending in [false, true] {
        if retry_pending {
            app.execute_command("git-refresh").unwrap();
            next_discovery(&operations);
        }
        let before = git_health(&app);
        let notifications = app.notifications.entries().to_vec();
        app.execute_command("detach").unwrap();
        assert!(matches!(
            app.take_persistent_exit_request(),
            Some(PersistentExitRequest::Detach)
        ));
        app.note_frontend_attached();
        assert_eq!(git_health(&app), before);
        assert_eq!(app.notifications.entries(), notifications);
        assert_eq!(refresh_available(&app), !retry_pending);
        assert!(operations.try_recv().is_err());
    }
}
