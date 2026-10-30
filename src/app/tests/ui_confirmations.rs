// SPDX-License-Identifier: MPL-2.0
use super::*;

pub(super) fn configure(app: &mut App, owner: &str, accept: &str) {
    let source = format!("ui: {{{owner}: {{accept: [{accept}], cancel: [F10]}}}}");
    let compiled =
        crate::keymap::configured::compile(&serde_yaml::from_str(&source).unwrap(), &app.keymap);
    assert!(compiled.errors.is_empty(), "{:?}", compiled.errors);
    app.keymap = compiled.keymap;
}

#[test]
fn configured_confirmation_reload_keeps_captured_buffer_and_revalidates_disk() {
    for accept in ["F8", "Esc"] {
        let root = temporary("configured-reload-confirmation");
        fs::create_dir_all(&root).unwrap();
        let path = root.join("file.txt");
        fs::write(&path, "before\n").unwrap();
        let mut app = App::new_in_isolated_project(
            &root,
            HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
                String::new(),
            ))))),
        )
        .unwrap();
        app.open_file(path.clone()).unwrap();
        let buffer = app.active().buffer;
        app.insert_text("unsaved ");
        configure(&mut app, "confirmation", accept);
        app.reload_file().unwrap();
        assert!(app.file_reload_confirmation.is_some());
        let accept_key = KeyStroke::parse(accept).unwrap();
        assert!(app.input_requires_presented_approval(&InputEvent::Key(accept_key)));
        assert!(
            !app.input_requires_presented_approval(&InputEvent::Key(KeyStroke::plain(
                KeyCode::Enter
            )))
        );
        key(&mut app, KeyCode::Enter, Modifiers::NONE);
        assert!(app.buffers[buffer].dirty);
        // A second disk change cannot consume the captured approval.
        fs::write(&path, "changed again\n").unwrap();
        app.handle_input(InputEvent::Key(accept_key)).unwrap();
        assert!(app.file_reload_confirmation.is_some());
        assert!(app.buffers[buffer].dirty);
        let snapshot = app
            .overlay_snapshots()
            .into_iter()
            .find(|s| s.kind == crate::snapshot::OverlayKind::Confirmation)
            .unwrap();
        assert_eq!(snapshot.actions[0].key_hint, accept);
        assert!(snapshot.message.unwrap().contains("F10 cancels"));
        app.handle_input(InputEvent::Key(accept_key)).unwrap();
        assert!(app.file_reload_confirmation.is_none());
        assert_eq!(app.buffers[buffer].to_string(), "changed again\n");
        assert!(!app.buffers[buffer].dirty);
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn configured_confirmation_directory_and_buffer_discard_keep_targets_and_cancel() {
    let root = temporary("configured-directory-confirmation");
    fs::create_dir_all(root.join("child")).unwrap();
    let mut app = App::new_in_isolated_project(
        &root,
        HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
            String::new(),
        ))))),
    )
    .unwrap();
    configure(&mut app, "confirmation", "F8");
    app.open_document(root.clone()).unwrap();
    let buffer = app.active().buffer;
    app.buffers[buffer].apply(&Transaction::insert(0, "unsaved"));
    app.refresh_directory().unwrap();
    assert!(app.directory_reload_confirmation.is_some());
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert!(app.buffers[buffer].dirty);
    key(&mut app, KeyCode::Function(10), Modifiers::NONE);
    assert!(app.directory_reload_confirmation.is_none() && app.buffers[buffer].dirty);
    app.refresh_directory().unwrap();
    key(&mut app, KeyCode::Function(8), Modifiers::NONE);
    assert!(!app.buffers[buffer].dirty);
    app.buffers[buffer].apply(&Transaction::insert(0, "unsaved"));
    app.directory_reload_confirmation = Some(DirectoryReloadConfirmation {
        buffer,
        destination: Some(root.join("child")),
        focus_entry: None,
    });
    key(&mut app, KeyCode::Function(8), Modifiers::NONE);
    assert_eq!(
        app.active_buffer().path.as_deref(),
        Some(root.join("child").as_path())
    );
    app.open_scratch_buffer();
    let scratch = app.active().buffer;
    app.insert_text("discard me");
    app.open_scratch_buffer();
    app.insert_text("keep me");
    app.buffer_discard_confirmation = Some(scratch);
    key(&mut app, KeyCode::Escape, Modifiers::NONE);
    assert!(app.buffer_discard_confirmation.is_some());
    key(&mut app, KeyCode::Function(8), Modifiers::NONE);
    assert!(app.buffers[scratch].to_string().is_empty());
    assert_eq!(app.active_buffer().to_string(), "keep me");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn configured_confirmation_leader_and_covering_owner_agree_with_approval_and_hints() {
    let mut app = App::new(Config::default(), None).unwrap();
    app.insert_text("keep");
    let buffer = app.active().buffer;
    app.open_navigator();
    app.list.as_mut().unwrap().filter = "query".into();
    app.buffer_discard_confirmation = Some(buffer);
    press(&mut app, ' ');
    assert_eq!(app.buffer_discard_confirmation, Some(buffer));
    app.list.as_mut().unwrap().filter.clear();
    press(&mut app, ' ');
    assert!(app.buffer_discard_confirmation.is_none());
    configure(&mut app, "confirmation", "Esc");
    app.buffer_discard_confirmation = Some(buffer);
    assert!(
        app.input_requires_presented_approval(&InputEvent::Key(KeyStroke::plain(KeyCode::Escape)))
    );
    app.git_branch_switch = Some(BranchSwitchConfirmation {
        repository: Repository::new("/example"),
        action: BranchSwitch::Checkout {
            branch: "topic".into(),
        },
        input: String::new(),
        cursor: 0,
    });
    assert!(
        !app.input_requires_presented_approval(&InputEvent::Key(KeyStroke::plain(KeyCode::Escape)))
    );
    key(&mut app, KeyCode::Escape, Modifiers::NONE);
    assert!(app.git_branch_switch.is_none());
    assert_eq!(app.buffer_discard_confirmation, Some(buffer));
}

#[test]
fn configured_stash_confirmation_reloads_hints_and_revalidates_captured_repository() {
    use crate::git::{GitOperation, GitServiceHandle};
    let root = temporary("configured-stash-confirmation");
    fs::create_dir_all(&root).unwrap();
    let repository = Repository::new(&root);
    let mut app = App::new_in_isolated_project(
        &root,
        HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
            String::new(),
        ))))),
    )
    .unwrap();
    let (service, operations) = GitServiceHandle::recording_for_test();
    app.attach_git_service(service);
    let _ = operations.recv_timeout(Duration::from_secs(1)).unwrap();
    app.git.attach(Some(repository.clone()));
    app.git_stash_confirmation = Some(GitStashConfirmation {
        repository: repository.clone(),
        mutation: StashMutation::Drop {
            oid: "a".repeat(40),
        },
        message: "Drop selected stash?".into(),
    });
    key(&mut app, KeyCode::Char('c'), Modifiers::CONTROL);
    assert!(
        app.git_stash_confirmation.is_some(),
        "default stash Ctrl-c stays inert"
    );
    configure(&mut app, "stash-confirmation", "F8");
    let snapshot = app
        .overlay_snapshots()
        .into_iter()
        .find(|s| s.kind == crate::snapshot::OverlayKind::Confirmation)
        .unwrap();
    assert!(
        snapshot
            .message
            .unwrap()
            .contains("F8 confirms.\nF10 cancels.")
    );
    assert_eq!(snapshot.actions[0].key_hint, "F8");
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert!(app.git_stash_confirmation.is_some() && operations.try_recv().is_err());
    let confirmation = app.git_stash_confirmation.clone();
    app.git.attach(Some(Repository::new(root.join("other"))));
    key(&mut app, KeyCode::Function(8), Modifiers::NONE);
    assert!(operations.try_recv().is_err());
    assert!(app.status.contains("different repository"));
    app.git.attach(Some(repository.clone()));
    app.git_stash_confirmation = confirmation.clone();
    key(&mut app, KeyCode::Function(10), Modifiers::NONE);
    assert!(app.git_stash_confirmation.is_none() && operations.try_recv().is_err());
    app.git_stash_confirmation = confirmation;
    app.handle_repeated_input(InputEvent::Key(KeyStroke::plain(KeyCode::Function(8))))
        .unwrap();
    assert!(app.git_stash_confirmation.is_none());
    let operation = operations.recv_timeout(Duration::from_secs(1)).unwrap();
    assert!(
        matches!(operation, GitOperation::Mutate { repository: target, mutation: GitMutation::Stash(StashMutation::Drop { oid }), .. } if target == repository && oid == "a".repeat(40)),
        "simple stash acceptance retains its existing repeat policy"
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn removed_confirmation_actions_leave_old_aliases_inert_and_hints_unbound() {
    let mut app = App::new(Config::default(), None).unwrap();
    app.insert_text("retained");
    let buffer = app.active().buffer;
    let source = serde_yaml::from_str("ui: {confirmation: {accept: [], cancel: []}}").unwrap();
    let compiled = crate::keymap::configured::compile(&source, &app.keymap);
    assert!(compiled.errors.is_empty());
    app.keymap = compiled.keymap;
    app.buffer_discard_confirmation = Some(buffer);
    for key in [
        KeyStroke::plain(KeyCode::Enter),
        KeyStroke::plain(KeyCode::Escape),
        KeyStroke::ctrl('c'),
        KeyStroke::char(' '),
    ] {
        assert!(!app.input_requires_presented_approval(&InputEvent::Key(key)));
        app.handle_input(InputEvent::Key(key)).unwrap();
        assert_eq!(app.buffer_discard_confirmation, Some(buffer));
    }
    assert_eq!(app.active_buffer().to_string(), "retained");
    let snapshot = app
        .overlay_snapshots()
        .into_iter()
        .find(|s| s.kind == crate::snapshot::OverlayKind::Confirmation)
        .unwrap();
    assert!(snapshot.actions.iter().all(|a| a.key_hint == "unbound"));
}

#[test]
fn configured_confirmation_snapshot_uses_dispatch_owner_when_decisions_overlap() {
    let mut app = App::new(Config::default(), None).unwrap();
    app.insert_text("retained");
    let buffer = app.active().buffer;
    let source = serde_yaml::from_str("ui: {confirmation: {accept: [F8], cancel: [F10]}, stash-confirmation: {accept: [F9], cancel: [F10]}}").unwrap();
    let compiled = crate::keymap::configured::compile(&source, &app.keymap);
    assert!(compiled.errors.is_empty());
    app.keymap = compiled.keymap;
    app.buffer_discard_confirmation = Some(buffer);
    app.git_stash_confirmation = Some(GitStashConfirmation {
        repository: Repository::new("/example"),
        mutation: StashMutation::Drop {
            oid: "a".repeat(40),
        },
        message: "Drop captured stash?".into(),
    });
    let snapshot = app
        .overlay_snapshots()
        .into_iter()
        .find(|s| s.kind == crate::snapshot::OverlayKind::Confirmation)
        .unwrap();
    assert_eq!(snapshot.title, "Drop stash");
    assert_eq!(snapshot.actions[0].key_hint, "F9");
    assert!(
        snapshot
            .message
            .unwrap()
            .contains("Drop captured stash?\nF9 confirms.")
    );
    key(&mut app, KeyCode::Function(8), Modifiers::NONE);
    assert!(app.git_stash_confirmation.is_some());
    key(&mut app, KeyCode::Function(10), Modifiers::NONE);
    assert!(app.git_stash_confirmation.is_none());
    let snapshot = app
        .overlay_snapshots()
        .into_iter()
        .find(|s| s.kind == crate::snapshot::OverlayKind::Confirmation)
        .unwrap();
    assert_eq!(snapshot.title, "Discard buffer changes");
    assert_eq!(snapshot.actions[0].key_hint, "F8");
    assert_eq!(app.buffers[buffer].to_string(), "retained");
}

#[test]
#[cfg(unix)]
fn configured_confirmation_covers_armed_terminal_in_rendering_and_dispatch() {
    let root = temporary("configured-confirmation-terminal");
    fs::create_dir_all(&root).unwrap();
    let mut app = App::new_in_isolated_project(
        &root,
        HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
            String::new(),
        ))))),
    )
    .unwrap();
    app.config.editor.auto_close_terminal = false;
    app.insert_text("keep until confirmed");
    let buffer = app.active().buffer;
    app.open_terminal(Some(terminal_fixture_command()));
    let id = app.active_terminal().unwrap();
    app.open_terminal_list();
    app.terminal_action_menu = Some(TerminalActionMenu {
        id,
        actions: vec![TerminalAction::ForceKill],
        selected: 0,
        close_armed: true,
    });
    configure(&mut app, "confirmation", "F8");
    app.buffer_discard_confirmation = Some(buffer);
    let snapshot = app
        .overlay_snapshots()
        .into_iter()
        .find(|s| s.kind == crate::snapshot::OverlayKind::Confirmation)
        .unwrap();
    assert_eq!(snapshot.title, "Discard buffer changes");
    assert_eq!(snapshot.actions[0].key_hint, "F8");
    assert!(snapshot.message.unwrap().contains("F8 confirms"));
    assert!(
        !app.input_requires_presented_approval(&InputEvent::Key(KeyStroke::plain(KeyCode::Enter)))
    );
    app.status_confirmation();
    assert!(app.status.contains("Discard changes"));
    key(&mut app, KeyCode::Function(8), Modifiers::NONE);
    assert!(app.buffers[buffer].to_string().is_empty());
    assert!(app.terminals.get(id).unwrap().live());
    assert!(app.terminal_action_menu.as_ref().unwrap().close_armed);
    let snapshot = app
        .overlay_snapshots()
        .into_iter()
        .find(|s| s.kind == crate::snapshot::OverlayKind::Confirmation)
        .unwrap();
    assert_eq!(snapshot.title, "Force kill terminal");
    assert_eq!(snapshot.actions[0].key_hint, "Enter");
    fs::remove_dir_all(root).unwrap();
}

pub(super) fn configure_text(app: &mut App) {
    let source = serde_yaml::from_str("ui: {text-confirmation: {accept: [F8], cancel: [F10], move-start: [F1], move-end: [F2], move-left: [F3], move-right: [F4], delete: [F6], backspace: [F7]}}").unwrap();
    let compiled = crate::keymap::configured::compile(&source, &app.keymap);
    assert!(compiled.errors.is_empty(), "{:?}", compiled.errors);
    app.keymap = compiled.keymap;
}

#[test]
fn configured_text_confirmation_preserves_graphemes_revision_and_exact_authorization() {
    for form in 0..3 {
        let mut app = App::new(Config::default(), None).unwrap();
        configure_text(&mut app);
        match form {
            0 => {
                app.git_branch_switch = Some(BranchSwitchConfirmation {
                    repository: Repository::new("/example"),
                    action: BranchSwitch::Checkout {
                        branch: "expected".into(),
                    },
                    input: String::new(),
                    cursor: 0,
                })
            }
            1 => {
                app.git_branch_deletion = Some(BranchDeletionConfirmation {
                    plan: BranchDeletionPlan {
                        branch: "expected".into(),
                        tip: "a".repeat(40),
                        upstream: None,
                        retaining_branches: vec![],
                        required_authorization: DeletionAuthorization::Typed,
                    },
                    cascade: None,
                    input: String::new(),
                    cursor: 0,
                })
            }
            _ => {
                app.git_worktree_removal = Some(WorktreeRemovalConfirmation {
                    plan: WorktreeRemovalPlan {
                        path: PathBuf::from("/example/linked"),
                        head: Some("a".repeat(40)),
                        branch: Some("expected".into()),
                        upstream: None,
                        detached_retained: false,
                        required_authorization: DeletionAuthorization::Typed,
                    },
                    session: None,
                    #[cfg(windows)]
                    reviewed_live: None,
                    input: String::new(),
                    cursor: 0,
                })
            }
        }
        let state = |app: &App| {
            if let Some(c) = &app.git_branch_switch {
                (c.input.clone(), c.cursor)
            } else if let Some(c) = &app.git_branch_deletion {
                (c.input.clone(), c.cursor)
            } else {
                let c = app.git_worktree_removal.as_ref().unwrap();
                (c.input.clone(), c.cursor)
            }
        };
        app.handle_input(InputEvent::Text("a👩‍💻e\u{301}".into()))
            .unwrap();
        let revision = app.confirmation_revision;
        key(&mut app, KeyCode::Backspace, Modifiers::NONE);
        assert_eq!(state(&app).0, "a👩‍💻e\u{301}");
        assert_eq!(
            app.confirmation_revision,
            revision.wrapping_add(1),
            "even unmatched editing intent invalidates an older frame"
        );
        key(&mut app, KeyCode::Function(7), Modifiers::NONE);
        assert_eq!(state(&app), ("a👩‍💻".into(), 4));
        key(&mut app, KeyCode::Function(3), Modifiers::NONE);
        assert_eq!(state(&app).1, 1);
        key(&mut app, KeyCode::Function(6), Modifiers::NONE);
        assert_eq!(state(&app), ("a".into(), 1));
        key(&mut app, KeyCode::Function(1), Modifiers::NONE);
        key(&mut app, KeyCode::Char('λ'), Modifiers::SUPER);
        press(&mut app, ' ');
        assert_eq!(
            state(&app),
            ("λ a".into(), 2),
            "literal input and spaces keep their structural meaning"
        );
        let accept = InputEvent::Key(KeyStroke::plain(KeyCode::Function(8)));
        assert!(app.input_requires_presented_approval(&accept));
        assert!(
            !app.input_requires_presented_approval(&InputEvent::Key(KeyStroke::plain(
                KeyCode::Enter
            )))
        );
        app.handle_input(accept).unwrap();
        assert!(app.status.contains("exact branch name"));
        assert_eq!(state(&app).0, "λ a");
        let snapshot = app
            .overlay_snapshots()
            .into_iter()
            .find(|s| s.kind == crate::snapshot::OverlayKind::Confirmation)
            .unwrap();
        assert_eq!(snapshot.actions[0].key_hint, "F8");
        assert_eq!(snapshot.actions[0].label, "confirm exact text");
        assert!(snapshot.message.unwrap().contains("F10 keeps"));
        key(&mut app, KeyCode::Function(10), Modifiers::NONE);
        assert!(
            app.git_branch_switch.is_none()
                && app.git_branch_deletion.is_none()
                && app.git_worktree_removal.is_none()
        );
    }
}

#[test]
#[cfg(unix)]
fn configured_text_confirmation_accept_keeps_deletion_authorization_and_captured_plan() {
    use crate::git::{GitOperation, GitServiceHandle};
    for worktree in [false, true] {
        for authorization in [DeletionAuthorization::Enter, DeletionAuthorization::Typed] {
            let root = temporary("configured-typed-authorization");
            fs::create_dir_all(&root).unwrap();
            let repository = Repository::new(&root);
            let mut app = App::new_in_isolated_project(
                &root,
                HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
                    String::new(),
                ))))),
            )
            .unwrap();
            let (service, operations) = GitServiceHandle::recording_for_test();
            app.attach_git_service(service);
            let _ = operations.recv_timeout(Duration::from_secs(1)).unwrap();
            app.git.attach(Some(repository.clone()));
            configure(&mut app, "text-confirmation", "Esc");
            let branch = "captured-branch".to_owned();
            if worktree {
                app.git_worktree_removal = Some(WorktreeRemovalConfirmation {
                    plan: WorktreeRemovalPlan {
                        path: root.join("linked"),
                        head: Some("a".repeat(40)),
                        branch: Some(branch.clone()),
                        upstream: None,
                        detached_retained: false,
                        required_authorization: authorization,
                    },
                    session: None,
                    input: String::new(),
                    cursor: 0,
                });
            } else {
                app.git_branch_deletion = Some(BranchDeletionConfirmation {
                    plan: BranchDeletionPlan {
                        branch: branch.clone(),
                        tip: "a".repeat(40),
                        upstream: None,
                        retaining_branches: vec!["main".into()],
                        required_authorization: authorization,
                    },
                    cascade: None,
                    input: String::new(),
                    cursor: 0,
                });
            }
            if authorization == DeletionAuthorization::Typed {
                key(&mut app, KeyCode::Escape, Modifiers::NONE);
                assert!(
                    operations.try_recv().is_err(),
                    "new acceptance still requires exact text"
                );
                app.handle_input(InputEvent::Text(branch.clone())).unwrap();
            }
            assert!(
                app.input_requires_presented_approval(&InputEvent::Key(KeyStroke::plain(
                    KeyCode::Escape
                )))
            );
            key(&mut app, KeyCode::Enter, Modifiers::NONE);
            assert!(operations.try_recv().is_err());
            key(&mut app, KeyCode::Escape, Modifiers::NONE);
            let operation = operations.recv_timeout(Duration::from_secs(1)).unwrap();
            match operation {
                GitOperation::Mutate {
                    repository: target,
                    mutation:
                        GitMutation::DeleteBranch {
                            plan,
                            authorization: granted,
                        },
                    ..
                } if !worktree => {
                    assert_eq!(target, repository);
                    assert_eq!(plan.branch, branch);
                    assert_eq!(plan.tip, "a".repeat(40));
                    assert_eq!(granted, authorization);
                }
                GitOperation::Mutate {
                    repository: target,
                    mutation:
                        GitMutation::RemoveWorktree {
                            plan,
                            authorization: granted,
                        },
                    ..
                } if worktree => {
                    assert_eq!(target, repository);
                    assert_eq!(plan.path, root.join("linked"));
                    assert_eq!(plan.branch, Some(branch));
                    assert_eq!(granted, authorization);
                }
                other => panic!("unexpected operation: {other:?}"),
            }
            fs::remove_dir_all(root).unwrap();
        }
    }
}
