// SPDX-License-Identifier: MPL-2.0

use super::*;

// -- Language servers --------------------------------------------------
//
// These drive the editor directly with the events a manager would emit, so
// no runtime and no language server is involved.

/// A unique temporary path per test, so nothing is written into the
/// repository and concurrent tests cannot collide.
pub(super) fn temporary(name: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    temporary_directory().join(format!("runyte-lsp-{}-{nanos}-{name}", std::process::id()))
}

/// An app whose active buffer looks like a saved Rust file, with a handle
/// whose queue the test owns.
pub(super) fn rust_app(text: &str) -> (App, PathBuf, tokio::sync::mpsc::Receiver<LspCommand>) {
    let mut app = App::new(Config::default(), None).unwrap();
    let path = temporary("a.rs");
    // `temporary` writes beside the system temp directory, so that is the
    // project as far as server-driven edits are concerned.
    app.project_root = temporary_directory();
    app.buffers[0].path = Some(path.clone());
    app.buffers[0].kind = crate::buffer::BufferKind::File;
    app.buffers[0].apply(&Transaction::insert(0, text));
    app.buffers[0].dirty = false;
    let (handle, queue) = crate::lsp::command_channel();
    app.lsp_workspace_allowed = true;
    app.attach_lsp(handle);
    (app, path, queue)
}

pub(super) fn tracked(app: &App, pending: PendingRequest) -> TrackedRequest {
    TrackedRequest::new(0, app.buffers[0].revision(), pending)
}

pub(super) fn ready_language(app: &mut App, language: &str, encoding: Encoding) {
    ready_language_with_capabilities(app, language, encoding, Capabilities::everything_for_test());
}

/// Like [`ready_language`], but with a caller-chosen set of advertised
/// capabilities — for tests of the gate itself, where a mock server that
/// supports everything would hide the behavior under test.
pub(super) fn ready_language_with_capabilities(
    app: &mut App,
    language: &str,
    encoding: Encoding,
    capabilities: Capabilities,
) {
    ready_language_with_sync(
        app,
        language,
        encoding,
        capabilities,
        DocumentSync {
            open_close: true,
            change: ChangeSync::Incremental,
            save: Some(true),
        },
    );
}

fn ready_language_with_sync(
    app: &mut App,
    language: &str,
    encoding: Encoding,
    capabilities: Capabilities,
    sync: DocumentSync,
) {
    app.apply_lsp_event(LspEvent::Ready {
        language: language.to_owned(),
        generation: 1,
        name: format!("mock-{language}-server"),
        encoding,
        sync,
        capabilities,
    });
}

pub(super) fn ready(app: &mut App, encoding: Encoding) {
    ready_language(app, "rust", encoding);
}

pub(super) fn drain(queue: &mut tokio::sync::mpsc::Receiver<LspCommand>) -> Vec<LspCommand> {
    let mut commands = Vec::new();
    while let Ok(command) = queue.try_recv() {
        commands.push(command);
    }
    commands
}
#[test]
fn status_stop_and_restart_clear_every_language_owned_transient() {
    let (mut app, _, mut queue) = rust_app("fn main() {}\n");
    ready(&mut app, Encoding::Utf8);
    drain(&mut queue);

    app.apply_lsp_event(LspEvent::Status {
        message: "indexing".to_owned(),
        error: false,
    });
    assert!(!app.status_error);
    app.apply_lsp_event(LspEvent::Status {
        message: "index failed".to_owned(),
        error: true,
    });
    assert!(app.status_error);

    app.completion = Some(CompletionState {
        items: vec![Completion {
            label: "main".to_owned(),
            filter_text: None,
            sort_text: None,
            detail: String::new(),
            kind: "function",
            insert: "main".to_owned(),
            edit: None,
            additional: Vec::new(),
        }],
        selected: 0,
        buffer: 0,
        anchor: 0,
        filter: String::new(),
        source: CompletionSource::Language,
        explicit_session: None,
    });
    app.signature = Some(SignatureState {
        signatures: Vec::new(),
    });
    app.hover = Some(HoverState {
        lines: vec!["documentation".to_owned()],
    });
    app.lsp_action_source = Some(ActionSource {
        buffer: 0,
        revision: app.buffers[0].revision(),
        documents: HashMap::new(),
        language: "rust".to_owned(),
        generation: 1,
    });
    app.lsp_actions
        .push(crate::lsp::ActionEntry::unresolved_for_test("fix"));
    app.list_actions.push(ListAction::CodeAction(0));
    app.pending_lsp_replies.push_back(LspCommand::EditApplied {
        language: "rust".to_owned(),
        generation: 1,
        id: serde_json::Value::from(1),
        applied: true,
    });
    app.pending_lsp_replies.push_back(LspCommand::EditApplied {
        language: "python".to_owned(),
        generation: 1,
        id: serde_json::Value::from(2),
        applied: true,
    });

    app.apply_lsp_event(LspEvent::Stopped {
        language: "rust".to_owned(),
        message: "server exited".to_owned(),
    });
    assert!(app.status_error);
    assert!(app.completion.is_none());
    assert!(app.signature.is_none());
    assert!(app.hover.is_none());
    assert!(app.lsp_action_source.is_none());
    assert!(app.lsp_actions.is_empty());
    assert!(app.list_actions.is_empty());
    assert!(matches!(
        app.pending_lsp_replies.front(),
        Some(LspCommand::EditApplied {
            language,
            id,
            applied: true,
            ..
        }) if language == "python" && id == &serde_json::Value::from(2)
    ));
    assert_eq!(app.pending_lsp_replies.len(), 1);

    ready(&mut app, Encoding::Utf8);
    app.completion = Some(CompletionState {
        items: Vec::new(),
        selected: 0,
        buffer: 0,
        anchor: 0,
        filter: String::new(),
        source: CompletionSource::Language,
        explicit_session: None,
    });
    app.signature = Some(SignatureState {
        signatures: Vec::new(),
    });
    app.hover = Some(HoverState { lines: Vec::new() });
    app.apply_lsp_event(LspEvent::Restarted {
        language: "rust".to_owned(),
    });
    assert!(app.completion.is_none());
    assert!(app.signature.is_none());
    assert!(app.hover.is_none());
}

#[test]
fn active_document_lsp_availability_tracks_server_lifecycle() {
    let (mut app, _path, mut queue) = rust_app("fn main() {}\n");
    assert_eq!(
        app.command_capabilities().lsp_document.reason(),
        Some("the rust language server is not ready")
    );

    ready(&mut app, Encoding::Utf8);
    assert!(app.command_capabilities().lsp_document.is_available());
    assert!(drain(&mut queue).iter().any(|command| matches!(
        command,
        LspCommand::Open { language, .. } if language == "rust"
    )));

    app.apply_lsp_event(LspEvent::Stopped {
        language: "rust".to_owned(),
        message: "mock server stopped".to_owned(),
    });
    assert_eq!(
        app.command_capabilities().lsp_document.reason(),
        Some("the rust language server is not ready")
    );
}

#[test]
fn restarting_a_live_server_retires_editor_state_before_reopening() {
    let (mut app, path, mut queue) = rust_app("fn main() {}\n");
    ready(&mut app, Encoding::Utf8);
    drain(&mut queue);
    app.apply_lsp_event(LspEvent::Diagnostics {
        language: "rust".to_owned(),
        path: path.clone(),
        version: Some(1),
        diagnostics: vec![diagnostic(0, 0, 2, "old")],
    });

    app.apply_lsp_event(LspEvent::Restarted {
        language: "rust".to_owned(),
    });
    assert!(!app.lsp_servers.contains_key("rust"));
    assert!(app.lsp_documents.is_empty());
    assert!(app.diagnostics.for_path(&path).is_empty());

    app.lsp_touch(0);
    assert!(matches!(
        drain(&mut queue).as_slice(),
        [LspCommand::Ensure { language }] if language == "rust"
    ));
}

#[test]
fn active_buffer_syntax_availability_tracks_current_tree() {
    let mut app = App::new(Config::default(), None).unwrap();
    app.buffers[0].path = Some(temporary("syntax.rs"));
    app.buffers[0].kind = crate::buffer::BufferKind::File;
    app.buffers[0].apply(&Transaction::insert(0, "fn main() {}\n"));
    app.reparse_whole(0);
    assert!(app.command_capabilities().syntax.is_available());

    app.syntax[0] = None;
    assert_eq!(
        app.command_capabilities().syntax.reason(),
        Some("syntax is unavailable for this buffer")
    );
}

#[test]
fn git_project_availability_distinguishes_missing_git_and_non_repository() {
    let mut app = App::new(Config::default(), None).unwrap();
    assert_eq!(
        app.command_capabilities().git_project.reason(),
        Some("no `git` executable was found")
    );

    app.ports
        .replace_git(Box::new(crate::git::GitCliProvider::new("git")));
    app.git_state.set_discovery_complete(true);
    assert_eq!(
        app.command_capabilities().git_project.reason(),
        Some("this project is not in a Git repository")
    );

    app.git.attach(Some(Repository::new(&app.project_root)));
    assert!(app.command_capabilities().git_project.is_available());

    let (service, operations) = GitServiceHandle::recording_for_test();
    app.attach_git_service(service);
    let operation = operations
        .recv_timeout(Duration::from_secs(1))
        .expect("Git discovery request");
    app.apply_git_service_event(GitServiceEvent::Completed {
        id: GitRequestId::from_raw(1),
        operation,
        result: Box::new(Err(crate::git::GitError::Malformed {
            command: "git rev-parse --show-toplevel".to_owned(),
            detail: "empty repository root".to_owned(),
        })),
        state: GitServiceState::Failed,
        coalesced: false,
    });
    assert!(
        app.command_capabilities()
            .git_project
            .reason()
            .is_some_and(|reason| reason.starts_with("Git repository discovery failed:"))
    );
}

fn diagnostic(row: u32, from: u32, to: u32, message: &str) -> crate::lsp::Diagnostic {
    crate::lsp::Diagnostic::new(lsp_types::Diagnostic {
        range: LspRange::new(LspPosition::new(row, from), LspPosition::new(row, to)),
        severity: Some(lsp_types::DiagnosticSeverity::ERROR),
        message: message.to_owned(),
        ..Default::default()
    })
}

fn edit(row: u32, from: u32, to: u32, text: &str) -> crate::lsp::TextEdit {
    crate::lsp::TextEdit {
        range: LspRange::new(LspPosition::new(row, from), LspPosition::new(row, to)),
        new_text: text.to_owned(),
    }
}

fn deliver_document_edits(
    app: &mut App,
    queue: &mut tokio::sync::mpsc::Receiver<LspCommand>,
    edits: Vec<DocumentEdit>,
    server_initiated: bool,
) {
    drain(queue);
    if server_initiated {
        app.apply_lsp_event(LspEvent::ApplyEdit {
            language: "rust".to_owned(),
            generation: 1,
            encoding: Encoding::Utf8,
            id: serde_json::json!(904),
            edits,
            skipped: 0,
        });
    } else {
        let request = tracked(
            app,
            PendingRequest::Edits {
                label: "formatted",
                path: PathBuf::new(),
            },
        );
        app.lsp_requests.insert(904, request);
        app.apply_lsp_event(LspEvent::Response {
            token: 904,
            response: Response::Edits {
                edits,
                skipped: 0,
                encoding: Encoding::Utf8,
            },
        });
    }
}

#[test]
fn document_edit_protections_are_warnings_through_both_request_paths() {
    for server_initiated in [false, true] {
        let (mut app, path, mut queue) = rust_app("original\n");
        ready(&mut app, Encoding::Utf8);
        deliver_document_edits(
            &mut app,
            &mut queue,
            vec![DocumentEdit {
                path,
                version: Some(999),
                edits: vec![edit(0, 0, 8, "changed")],
            }],
            server_initiated,
        );
        assert_eq!(
            app.notifications.entries()[0].severity,
            NotificationSeverity::Warning
        );

        let (mut app, path, mut queue) = rust_app("original\n");
        ready(&mut app, Encoding::Utf8);
        app.buffers[0].kind = BufferKind::Help;
        deliver_document_edits(
            &mut app,
            &mut queue,
            vec![DocumentEdit {
                path,
                version: None,
                edits: vec![edit(0, 0, 8, "changed")],
            }],
            server_initiated,
        );
        assert_eq!(
            app.notifications.entries()[0].severity,
            NotificationSeverity::Warning
        );
    }
}

#[test]
fn malformed_and_unopenable_document_edits_are_errors_through_both_request_paths() {
    for server_initiated in [false, true] {
        let (mut app, path, mut queue) = rust_app("original\n");
        ready(&mut app, Encoding::Utf8);
        deliver_document_edits(
            &mut app,
            &mut queue,
            vec![DocumentEdit {
                path,
                version: None,
                edits: vec![edit(8, 0, 0, "changed")],
            }],
            server_initiated,
        );
        assert_eq!(
            app.notifications.entries()[0].severity,
            NotificationSeverity::Error
        );

        let (mut app, path, mut queue) = rust_app("original\n");
        ready(&mut app, Encoding::Utf8);
        deliver_document_edits(
            &mut app,
            &mut queue,
            vec![DocumentEdit {
                path,
                version: None,
                edits: vec![edit(0, 0, 8, "first"), edit(0, 0, 8, "second")],
            }],
            server_initiated,
        );
        assert_eq!(
            app.notifications.entries()[0].severity,
            NotificationSeverity::Error
        );

        let (mut app, _path, mut queue) = rust_app("original\n");
        ready(&mut app, Encoding::Utf8);
        let binary = temporary("unopenable-edit.rs");
        fs::write(&binary, [0xff, 0xfe]).unwrap();
        deliver_document_edits(
            &mut app,
            &mut queue,
            vec![DocumentEdit {
                path: binary.clone(),
                version: None,
                edits: vec![edit(0, 0, 0, "changed")],
            }],
            server_initiated,
        );
        assert_eq!(
            app.notifications.entries()[0].severity,
            NotificationSeverity::Error
        );
        fs::remove_file(binary).unwrap();
    }
}

#[test]
fn a_document_is_announced_only_once_the_handshake_lands() {
    let (mut app, path, mut queue) = rust_app("fn main() {}\n");

    // Before `Ready`, the editor may only ask for the server to start:
    // the position encoding a `didChange` needs is not known yet.
    let before = drain(&mut queue);
    assert!(matches!(
        before.as_slice(),
        [LspCommand::Ensure { language }] if language == "rust"
    ));

    ready(&mut app, Encoding::Utf8);
    let after = drain(&mut queue);
    assert!(
        after.iter().any(|command| matches!(
            command,
            LspCommand::Open { path: opened, .. } if opened == &path
        )),
        "{after:?}"
    );
}

#[test]
fn extensionless_shebang_edits_switch_syntax_and_lsp_without_cross_language_changes() {
    let path = temporary("script");
    fs::write(&path, "echo plain\n").unwrap();
    let mut app = App::new(Config::default(), Some(path.clone())).unwrap();
    let buffer_id = app.active().buffer;
    assert_eq!(syntax_language_name(&app, buffer_id), None);

    let (handle, mut queue) = crate::lsp::command_channel();
    app.lsp_workspace_allowed = true;
    app.attach_lsp(handle);
    assert!(drain(&mut queue).is_empty());
    ready_language(&mut app, "bash", Encoding::Utf8);
    drain(&mut queue);

    let shebang = "#!/bin/bash\n";
    assert!(app.apply_to_buffer(buffer_id, &Transaction::insert(0, shebang)));
    assert_eq!(syntax_language_name(&app, buffer_id), Some("bash"));
    let commands = drain(&mut queue);
    assert!(commands.iter().any(|command| matches!(
        command,
        LspCommand::Open { language, path: opened, .. }
            if language == "bash" && opened == &path
    )));
    assert!(
        !commands
            .iter()
            .any(|command| matches!(command, LspCommand::Change { .. }))
    );

    app.apply_lsp_event(LspEvent::Diagnostics {
        language: "bash".to_owned(),
        path: path.clone(),
        version: None,
        diagnostics: vec![diagnostic(0, 0, 2, "old bash diagnostic")],
    });
    app.lsp_requests
        .insert(7, tracked(&app, PendingRequest::Hover));

    assert!(app.apply_to_buffer(buffer_id, &Transaction::delete(0, shebang.chars().count())));
    assert_eq!(syntax_language_name(&app, buffer_id), None);
    assert!(app.diagnostics.for_path(&path).is_empty());
    assert!(!app.lsp_requests.contains_key(&7));
    let commands = drain(&mut queue);
    assert!(commands.iter().any(|command| matches!(
        command,
        LspCommand::Close { language, path: closed }
            if language == "bash" && closed == &path
    )));
    assert!(
        !commands
            .iter()
            .any(|command| matches!(command, LspCommand::Change { .. }))
    );

    app.undo();
    assert_eq!(syntax_language_name(&app, buffer_id), Some("bash"));
    let commands = drain(&mut queue);
    assert!(commands.iter().any(|command| matches!(
        command,
        LspCommand::Open { language, path: opened, .. }
            if language == "bash" && opened == &path
    )));
    assert!(
        !commands
            .iter()
            .any(|command| matches!(command, LspCommand::Change { .. }))
    );

    app.redo();
    assert_eq!(syntax_language_name(&app, buffer_id), None);
    let commands = drain(&mut queue);
    assert!(commands.iter().any(|command| matches!(
        command,
        LspCommand::Close { language, path: closed }
            if language == "bash" && closed == &path
    )));
    assert!(
        !commands
            .iter()
            .any(|command| matches!(command, LspCommand::Change { .. }))
    );
    fs::remove_file(path).unwrap();
}

#[test]
fn a_fixed_shell_extension_keeps_one_language_document_when_the_shebang_changes() {
    let path = temporary("fixed.sh");
    fs::write(&path, "#!/bin/bash\necho fixed\n").unwrap();
    let mut app = App::new(Config::default(), Some(path.clone())).unwrap();
    let buffer_id = app.active().buffer;
    assert_eq!(syntax_language_name(&app, buffer_id), Some("bash"));

    let (handle, mut queue) = crate::lsp::command_channel();
    app.lsp_workspace_allowed = true;
    app.attach_lsp(handle);
    ready_language(&mut app, "bash", Encoding::Utf8);
    drain(&mut queue);

    assert!(app.apply_to_buffer(buffer_id, &Transaction::delete(0, "#!/bin/bash\n".len())));
    assert_eq!(syntax_language_name(&app, buffer_id), Some("bash"));
    let commands = drain(&mut queue);
    assert!(commands.iter().any(|command| matches!(
        command,
        LspCommand::Change { language, path: changed, .. }
            if language == "bash" && changed == &path
    )));
    assert!(
        !commands
            .iter()
            .any(|command| matches!(command, LspCommand::Open { .. } | LspCommand::Close { .. }))
    );
    fs::remove_file(path).unwrap();
}

#[test]
fn closing_a_buffer_closes_its_language_server_document() {
    let (mut app, path, mut queue) = rust_app("fn main() {}\n");
    ready(&mut app, Encoding::Utf16);
    drain(&mut queue);
    app.hover = Some(HoverState {
        lines: vec!["stale hover".to_owned()],
    });
    app.signature = Some(SignatureState {
        signatures: vec![SignatureLine {
            label: "fn stale()".to_owned(),
            documentation: String::new(),
            active_parameter: None,
        }],
    });
    app.completion = Some(CompletionState {
        items: Vec::new(),
        selected: 0,
        buffer: 0,
        anchor: 0,
        filter: String::new(),
        source: CompletionSource::Language,
        explicit_session: None,
    });
    app.lsp_requests
        .insert(73, tracked(&app, PendingRequest::Hover));
    app.apply_lsp_event(LspEvent::Diagnostics {
        language: "rust".to_owned(),
        path: path.clone(),
        version: None,
        diagnostics: vec![diagnostic(0, 0, 2, "stale diagnostic")],
    });
    assert!(!app.diagnostics.for_path(&path).is_empty());

    app.open_buffer_picker();
    key(&mut app, KeyCode::Tab, Modifiers::NONE);
    key(&mut app, KeyCode::Enter, Modifiers::NONE);

    assert!(drain(&mut queue).into_iter().any(|command| {
        matches!(command, LspCommand::Close { path: closed, .. } if closed == path)
    }));
    assert!(app.hover.is_none());
    assert!(app.signature.is_none());
    assert!(app.completion.is_none());
    assert!(!app.lsp_requests.contains_key(&73));
    assert!(app.diagnostics.for_path(&path).is_empty());
}

#[test]
fn filesystem_rename_reopens_the_same_language_at_a_savable_new_path() {
    let directory = temporary("rename-same-language");
    fs::create_dir_all(&directory).unwrap();
    let old_path = directory.join("old.sh");
    let new_path = directory.join("new.sh");
    fs::write(&old_path, "echo same\n").unwrap();
    let mut app = App::new(Config::default(), Some(old_path.clone())).unwrap();
    let buffer_id = app.active().buffer;
    let (handle, mut queue) = crate::lsp::command_channel();
    app.lsp_workspace_allowed = true;
    app.attach_lsp(handle);
    ready_language(&mut app, "bash", Encoding::Utf8);
    drain(&mut queue);

    fs::rename(&old_path, &new_path).unwrap();
    let report = ApplyReport {
        recovery: Vec::new(),
        applied: vec![FsOperation::Rename {
            from: PathBuf::from("old.sh"),
            to: PathBuf::from("new.sh"),
            kind: EntryKind::File,
        }],
    };
    assert_eq!(
        app.reconcile_applied_filesystem(&directory, buffer_id, &report, true),
        None
    );

    assert_eq!(
        app.buffers[buffer_id].path.as_deref(),
        Some(new_path.as_path())
    );
    assert_eq!(
        app.buffers[buffer_id].path.as_ref().unwrap().as_os_str(),
        new_path.as_os_str(),
        "the buffer identity must not retain a trailing separator"
    );
    assert_eq!(syntax_language_name(&app, buffer_id), Some("bash"));
    let commands = drain(&mut queue);
    assert!(matches!(
        commands.as_slice(),
        [
            LspCommand::Close { language: closed_language, path: closed },
            LspCommand::Open { language: opened_language, path: opened, .. },
        ] if closed_language == "bash"
            && opened_language == "bash"
            && closed == &old_path
            && opened == &new_path
    ));

    assert!(app.apply_to_buffer(
        buffer_id,
        &Transaction::insert(app.buffers[buffer_id].len_chars(), "# later\n")
    ));
    assert!(matches!(
        drain(&mut queue).as_slice(),
        [LspCommand::Change { language, path, .. }]
            if language == "bash" && path == &new_path
    ));
    app.save(None, false).unwrap();
    assert!(!app.status_error, "{}", app.status);
    assert!(!app.buffers[buffer_id].dirty);
    assert_eq!(
        fs::read_to_string(&new_path).unwrap(),
        "echo same\n# later\n"
    );
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn filesystem_rename_reinfers_language_before_future_changes() {
    let directory = temporary("rename-changed-language");
    fs::create_dir_all(&directory).unwrap();
    let old_path = directory.join("script.sh");
    let new_path = directory.join("script.go");
    fs::write(&old_path, "echo changed\n").unwrap();
    let mut app = App::new(Config::default(), Some(old_path.clone())).unwrap();
    let buffer_id = app.active().buffer;
    let (handle, mut queue) = crate::lsp::command_channel();
    app.lsp_workspace_allowed = true;
    app.attach_lsp(handle);
    ready_language(&mut app, "bash", Encoding::Utf8);
    ready_language(&mut app, "go", Encoding::Utf8);
    drain(&mut queue);

    fs::rename(&old_path, &new_path).unwrap();
    let report = ApplyReport {
        recovery: Vec::new(),
        applied: vec![FsOperation::Rename {
            from: PathBuf::from("script.sh"),
            to: PathBuf::from("script.go"),
            kind: EntryKind::File,
        }],
    };
    assert_eq!(
        app.reconcile_applied_filesystem(&directory, buffer_id, &report, true),
        None
    );

    assert_eq!(syntax_language_name(&app, buffer_id), Some("go"));
    let commands = drain(&mut queue);
    assert!(matches!(
        commands.as_slice(),
        [
            LspCommand::Close { language: closed_language, path: closed },
            LspCommand::Open { language: opened_language, path: opened, .. },
        ] if closed_language == "bash"
            && opened_language == "go"
            && closed == &old_path
            && opened == &new_path
    ));

    assert!(app.apply_to_buffer(
        buffer_id,
        &Transaction::insert(app.buffers[buffer_id].len_chars(), "// later\n")
    ));
    assert!(matches!(
        drain(&mut queue).as_slice(),
        [LspCommand::Change { language, path, .. }]
            if language == "go" && path == &new_path
    ));
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn editing_sends_incremental_changes_in_the_negotiated_encoding() {
    let (mut app, _, mut queue) = rust_app("let é = 1;\n");
    ready(&mut app, Encoding::Utf8);
    drain(&mut queue);

    // Insert at the end of the first line, past a two-byte character.
    set_cursor(&mut app, 0, 10);
    press(&mut app, 'i');
    type_text(&mut app, "!");

    let commands = drain(&mut queue);
    let Some(LspCommand::Change {
        version, changes, ..
    }) = commands.into_iter().next_back()
    else {
        panic!("an edit must produce a didChange");
    };
    assert_eq!(version, 2, "versions must advance with every change");
    assert_eq!(changes.len(), 1);
    let range = changes[0].range.expect("incremental sync sends ranges");
    // Ten characters, one of which is two bytes.
    assert_eq!(range.start.character, 11);
    assert_eq!(changes[0].text, "!");
}

#[test]
fn undo_resynchronizes_the_whole_document() {
    let (mut app, _, mut queue) = rust_app("abc\n");
    ready(&mut app, Encoding::Utf8);
    drain(&mut queue);
    press(&mut app, 'i');
    type_text(&mut app, "x");
    drain(&mut queue);

    key(&mut app, KeyCode::Escape, Modifiers::NONE);
    press(&mut app, 'u');

    let commands = drain(&mut queue);
    let Some(LspCommand::Change { changes, .. }) = commands.into_iter().next_back() else {
        panic!("undo must tell the server what the document became");
    };
    assert!(
        changes[0].range.is_none(),
        "undo produces no transaction to derive a delta from"
    );
    assert_eq!(changes[0].text, "abc\n");
}

#[test]
fn reload_resynchronizes_the_whole_document() {
    let (mut app, path, mut queue) = rust_app("old\n");
    ready(&mut app, Encoding::Utf8);
    drain(&mut queue);
    fs::write(&path, "new from disk\n").unwrap();

    app.execute_command("reload").unwrap();

    let commands = drain(&mut queue);
    let Some(LspCommand::Change { changes, .. }) = commands.into_iter().next_back() else {
        panic!("reload must tell the server what the document became");
    };
    assert!(changes[0].range.is_none());
    assert_eq!(changes[0].text, "new from disk\n");
    fs::remove_file(path).unwrap();
}

#[test]
fn reload_retires_an_extensionless_document_when_its_shebang_disappears() {
    let path = temporary("reload-script");
    fs::write(&path, "#!/bin/bash\necho old\n").unwrap();
    let mut app = App::new(Config::default(), Some(path.clone())).unwrap();
    let buffer_id = app.active().buffer;
    assert_eq!(syntax_language_name(&app, buffer_id), Some("bash"));
    let (handle, mut queue) = crate::lsp::command_channel();
    app.lsp_workspace_allowed = true;
    app.attach_lsp(handle);
    ready_language(&mut app, "bash", Encoding::Utf8);
    drain(&mut queue);

    fs::write(&path, "echo plain\n").unwrap();
    app.reload_file().unwrap();

    assert_eq!(syntax_language_name(&app, buffer_id), None);
    let commands = drain(&mut queue);
    assert!(commands.iter().any(|command| matches!(
        command,
        LspCommand::Close { language, path: closed }
            if language == "bash" && closed == &path
    )));
    assert!(
        !commands
            .iter()
            .any(|command| matches!(command, LspCommand::Change { .. }))
    );
    fs::remove_file(path).unwrap();
}

#[test]
fn save_as_closes_the_old_path_before_recomputing_language_identity() {
    let directory = temporary("save-as-language");
    fs::create_dir_all(&directory).unwrap();
    let old_path = directory.join("script.sh");
    let new_path = directory.join("script");
    fs::write(&old_path, "echo plain\n").unwrap();
    let mut app = App::new(Config::default(), Some(old_path.clone())).unwrap();
    let buffer_id = app.active().buffer;
    let (handle, mut queue) = crate::lsp::command_channel();
    app.lsp_workspace_allowed = true;
    app.attach_lsp(handle);
    ready_language(&mut app, "bash", Encoding::Utf8);
    drain(&mut queue);

    app.save(Some(new_path.clone()), false).unwrap();

    assert_eq!(
        app.buffers[buffer_id].path.as_deref(),
        Some(new_path.as_path())
    );
    assert_eq!(syntax_language_name(&app, buffer_id), None);
    let commands = drain(&mut queue);
    assert!(commands.iter().any(|command| matches!(
        command,
        LspCommand::Close { language, path }
            if language == "bash" && path == &old_path
    )));
    assert!(!commands.iter().any(|command| matches!(
        command,
        LspCommand::Change { .. } | LspCommand::Open { .. } | LspCommand::Save { .. }
    )));
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn save_as_reopens_a_stable_language_at_the_new_document_uri() {
    let directory = temporary("save-as-stable-language");
    fs::create_dir_all(&directory).unwrap();
    let old_path = directory.join("old.sh");
    let new_path = directory.join("new.sh");
    fs::write(&old_path, "echo stable\n").unwrap();
    let mut app = App::new(Config::default(), Some(old_path.clone())).unwrap();
    let (handle, mut queue) = crate::lsp::command_channel();
    app.lsp_workspace_allowed = true;
    app.attach_lsp(handle);
    ready_language(&mut app, "bash", Encoding::Utf8);
    drain(&mut queue);

    app.save(Some(new_path.clone()), false).unwrap();

    let commands = drain(&mut queue);
    assert!(matches!(
        commands.as_slice(),
        [
            LspCommand::Close { language: closed_language, path: closed },
            LspCommand::Open { language: opened_language, path: opened, .. },
            LspCommand::Save { language: saved_language, path: saved, .. },
        ] if closed_language == "bash"
            && opened_language == "bash"
            && saved_language == "bash"
            && closed == &old_path
            && opened == &new_path
            && saved == &new_path
    ));
    assert_eq!(
        syntax_language_name(&app, app.active().buffer),
        Some("bash")
    );
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn failed_save_as_preserves_syntax_and_the_open_lsp_document() {
    let directory = temporary("failed-save-as-language");
    fs::create_dir_all(&directory).unwrap();
    let old_path = directory.join("script.sh");
    let requested = directory.join("missing").join("script.go");
    fs::write(&old_path, "echo stable\n").unwrap();
    let mut app = App::new(Config::default(), Some(old_path.clone())).unwrap();
    let buffer_id = app.active().buffer;
    let (handle, mut queue) = crate::lsp::command_channel();
    app.lsp_workspace_allowed = true;
    app.attach_lsp(handle);
    ready_language(&mut app, "bash", Encoding::Utf8);
    drain(&mut queue);

    app.save(Some(requested), false).unwrap();

    assert!(app.status_error);
    assert_eq!(
        app.buffers[buffer_id].path.as_deref(),
        Some(old_path.as_path())
    );
    assert_eq!(syntax_language_name(&app, buffer_id), Some("bash"));
    let document = app.lsp_documents.get(&buffer_id).unwrap();
    assert_eq!(document.language, "bash");
    assert_eq!(document.path, old_path);
    assert!(drain(&mut queue).is_empty());
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn diagnostics_render_as_signs_spans_and_an_inline_message() {
    let (mut app, path, _queue) = rust_app("let x = 1;\nlet y = 2;\n");
    ready(&mut app, Encoding::Utf8);
    app.apply_lsp_event(LspEvent::Diagnostics {
        language: "rust".to_owned(),
        path: path.clone(),
        version: None,
        diagnostics: vec![diagnostic(1, 4, 5, "unused variable")],
    });

    assert_eq!(app.row_severity(0, 1), Some(crate::lsp::Severity::Error));
    assert_eq!(app.row_severity(0, 0), None);
    let spans = app.diagnostic_spans(0, 1);
    assert_eq!(spans.len(), 1);
    // Row 1 starts at offset 11, so character 4 is offset 15.
    assert_eq!((spans[0].0, spans[0].1), (15, 16));
    let (message, _) = app.inline_diagnostic(0, 1).unwrap();
    assert!(message.contains("unused variable"), "{message}");
    assert!(app.lsp_summary().unwrap().contains("1E"));
}

#[test]
fn diagnostics_picker_uses_the_publishing_servers_encoding() {
    let (mut app, path, _queue) = rust_app("aéx\n");
    ready(&mut app, Encoding::Utf8);
    app.apply_lsp_event(LspEvent::Diagnostics {
        language: "rust".to_owned(),
        path,
        version: Some(app.lsp_documents[&0].version),
        diagnostics: vec![diagnostic(0, 3, 4, "utf-8 location")],
    });
    app.open_diagnostics_picker();
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert_eq!(cursor(&app), Position::new(0, 2));
}

#[test]
fn stale_or_unowned_diagnostic_publications_are_ignored() {
    let (mut app, path, mut queue) = rust_app("fn main() {}\n");
    ready(&mut app, Encoding::Utf8);
    drain(&mut queue);
    let version = app.lsp_documents[&0].version;

    app.apply_lsp_event(LspEvent::Diagnostics {
        language: "rust".to_owned(),
        path: path.clone(),
        version: Some(version),
        diagnostics: vec![diagnostic(0, 0, 2, "current")],
    });
    assert_eq!(app.diagnostics.for_path(&path)[0].message, "current");

    app.apply_lsp_event(LspEvent::Diagnostics {
        language: "rust".to_owned(),
        path: path.clone(),
        version: Some(version - 1),
        diagnostics: vec![diagnostic(0, 0, 2, "stale")],
    });
    assert_eq!(app.diagnostics.for_path(&path)[0].message, "current");

    let outside = temporary("unowned.rs");
    app.apply_lsp_event(LspEvent::Diagnostics {
        language: "rust".to_owned(),
        path: outside.clone(),
        version: None,
        diagnostics: vec![diagnostic(0, 0, 2, "unowned")],
    });
    assert!(app.diagnostics.for_path(&outside).is_empty());
}

#[test]
fn no_change_sync_still_advances_the_local_document_version() {
    let (mut app, path, mut queue) = rust_app("fn main() {}\n");
    ready_language_with_sync(
        &mut app,
        "rust",
        Encoding::Utf8,
        Capabilities::everything_for_test(),
        DocumentSync::default(),
    );
    drain(&mut queue);
    let old_version = app.lsp_documents[&0].version;
    let before = app.buffers[0].text().clone();
    let transaction = Transaction::insert(0, "// local\n");
    app.buffers[0].apply(&transaction);
    app.lsp_change(0, &before, &transaction);

    assert_eq!(app.lsp_documents[&0].version, old_version + 1);
    assert!(drain(&mut queue).is_empty());
    app.apply_lsp_event(LspEvent::Diagnostics {
        language: "rust".to_owned(),
        path: path.clone(),
        version: Some(old_version),
        diagnostics: vec![diagnostic(0, 0, 2, "stale")],
    });
    assert!(app.diagnostics.for_path(&path).is_empty());
}

#[test]
fn a_rejected_change_forces_full_resync_before_the_next_request() {
    let (mut app, path, mut queue) = rust_app("fn main() {}\n");
    ready(&mut app, Encoding::Utf8);
    drain(&mut queue);
    for _ in 0..crate::lsp::COMMAND_CAPACITY {
        assert!(app.lsp_send(LspCommand::Status));
    }
    let old_version = app.lsp_documents[&0].version;
    let before = app.buffers[0].text().clone();
    let transaction = Transaction::insert(0, "// local\n");
    app.buffers[0].apply(&transaction);
    app.lsp_change(0, &before, &transaction);
    assert!(app.lsp_documents[&0].desynced);
    assert_eq!(app.lsp_documents[&0].version, old_version + 1);

    app.apply_lsp_event(LspEvent::Diagnostics {
        language: "rust".to_owned(),
        path: path.clone(),
        version: None,
        diagnostics: vec![diagnostic(0, 0, 2, "untrusted while desynced")],
    });
    assert!(app.diagnostics.for_path(&path).is_empty());
    drain(&mut queue);

    app.lsp_hover();
    let commands = drain(&mut queue);
    assert!(matches!(
        commands.first(),
        Some(LspCommand::Change { changes, version, .. })
            if *version == old_version + 1
                && matches!(changes.as_slice(), [TextDocumentContentChangeEvent { range: None, .. }])
    ));
    assert!(commands.iter().any(|command| matches!(
        command,
        LspCommand::Request { kind, .. } if matches!(kind.as_ref(), RequestKind::Hover(_))
    )));
    assert!(!app.lsp_documents[&0].desynced);
}

#[test]
fn a_stopped_server_drops_its_diagnostics_without_touching_the_buffer() {
    let (mut app, path, _queue) = rust_app("let x = 1;\n");
    ready(&mut app, Encoding::Utf8);
    app.apply_lsp_event(LspEvent::Diagnostics {
        language: "rust".to_owned(),
        path: path.clone(),
        version: None,
        diagnostics: vec![diagnostic(0, 4, 5, "unused variable")],
    });
    let before = text(&app);

    app.apply_lsp_event(LspEvent::Stopped {
        language: "rust".to_owned(),
        message: "mock-analyzer stopped: exited".to_owned(),
    });

    assert!(app.diagnostics.is_empty());
    assert_eq!(text(&app), before, "a lost server must not lose text");
    assert!(app.status_error);
    assert!(app.lsp_summary().is_none());
    // Editing still works with no server behind it.
    press(&mut app, 'i');
    type_text(&mut app, "z");
    assert!(text(&app).starts_with('z'));
}

#[test]
fn a_server_that_never_advertised_signature_help_never_gets_the_request() {
    let (mut app, _path, mut queue) = rust_app("fn f(a: i32) {}\nfn main() { f(\n");
    ready_language_with_capabilities(
        &mut app,
        "rust",
        Encoding::Utf8,
        Capabilities {
            signature_help: false,
            ..Capabilities::everything_for_test()
        },
    );
    drain(&mut queue);

    app.lsp_signature(SignatureContext::default());

    assert!(
        drain(&mut queue)
            .into_iter()
            .all(|command| !matches!(command, LspCommand::Request { .. })),
        "an unsupported request must never reach the manager"
    );
    assert!(
        app.status
            .contains("rust language server does not support signature help"),
        "{}",
        app.status
    );
    // A silent no-op would leave this unchanged; the person still sees
    // why nothing happened, but nothing about it is worth reading back
    // later, so the count stays at zero.
    assert_eq!(
        app.unread_notification_counts(),
        NotificationCounts::default()
    );
}

/// What typing asked the server for, by label, so a test can say what a
/// keystroke produced without matching the whole command.
fn requested_kinds(queue: &mut tokio::sync::mpsc::Receiver<LspCommand>) -> Vec<&'static str> {
    drain(queue)
        .into_iter()
        .filter_map(|command| match command {
            LspCommand::Request { kind, .. } => Some(kind.label()),
            _ => None,
        })
        .collect()
}

/// A signature popup already on screen, so a retrigger character has
/// something to re-ask about.
fn showing_signature(app: &mut App) {
    app.signature = Some(SignatureState {
        signatures: vec![SignatureLine {
            label: "fn f(a: i32)".to_owned(),
            documentation: String::new(),
            active_parameter: None,
        }],
    });
}

#[test]
fn completion_is_asked_for_on_the_servers_own_trigger_characters() {
    let (mut app, _path, mut queue) = rust_app("fn main() {}\n");
    ready_language_with_capabilities(
        &mut app,
        "rust",
        Encoding::Utf8,
        Capabilities {
            completion_triggers: vec!['"', '['],
            signature_triggers: Vec::new(),
            signature_retriggers: Vec::new(),
            ..Capabilities::everything_for_test()
        },
    );
    press(&mut app, 'i');
    drain(&mut queue);

    // `.` and `:` are what Runyte used to ask on; this server named
    // neither, so typing them costs no round trip.
    type_text(&mut app, ".");
    assert!(
        requested_kinds(&mut queue).is_empty(),
        "a character the server did not name must not ask it anything"
    );
    type_text(&mut app, ":");
    assert!(requested_kinds(&mut queue).is_empty());

    for character in ["\"", "["] {
        type_text(&mut app, character);
        assert_eq!(
            requested_kinds(&mut queue),
            ["completion"],
            "typing {character} is this server's own cue"
        );
    }
}

#[test]
fn signature_help_retriggers_on_the_closing_delimiter_the_server_named() {
    let (mut app, _path, mut queue) = rust_app("fn main() {}\n");
    ready_language_with_capabilities(
        &mut app,
        "rust",
        Encoding::Utf8,
        Capabilities {
            signature_triggers: vec!['('],
            signature_retriggers: vec![')'],
            ..Capabilities::everything_for_test()
        },
    );
    press(&mut app, 'i');
    drain(&mut queue);

    // A retrigger character is inert until a popup is showing.
    type_text(&mut app, ")");
    assert!(requested_kinds(&mut queue).is_empty());
    type_text(&mut app, "(");
    assert_eq!(requested_kinds(&mut queue), ["signature help"]);
    // This server did not name `,`, though Runyte used to ask on it.
    // (The `(` above asked with no popup showing, so it was not a
    // retrigger; the closing `)` below is checked in full.)
    type_text(&mut app, ",");
    assert!(requested_kinds(&mut queue).is_empty());

    // The inner `)` of `f(g(a), b)` asks again rather than dismissing:
    // only the server knows the caret is back inside `f`, and only the
    // context tells it this `)` closed an inner call.
    showing_signature(&mut app);
    type_text(&mut app, ")");
    let context = drain(&mut queue)
        .into_iter()
        .find_map(|command| match command {
            LspCommand::Request { kind, .. } => match *kind {
                RequestKind::SignatureHelp { context, .. } => Some(context),
                _ => None,
            },
            _ => None,
        })
        .expect("a retrigger character asks again");
    assert_eq!(context.trigger, Some(')'));
    assert!(
        context.retrigger,
        "a server cannot honour its own retrigger characters without being told"
    );
    assert!(
        app.signature.is_some(),
        "the popup stays until the answer replaces or clears it"
    );
}

#[test]
fn a_server_that_named_no_retrigger_character_still_closes_the_popup_locally() {
    let (mut app, _path, mut queue) = rust_app("fn main() {}\n");
    ready_language_with_capabilities(
        &mut app,
        "rust",
        Encoding::Utf8,
        Capabilities {
            signature_triggers: vec!['('],
            signature_retriggers: Vec::new(),
            ..Capabilities::everything_for_test()
        },
    );
    press(&mut app, 'i');
    showing_signature(&mut app);
    drain(&mut queue);

    type_text(&mut app, ")");
    assert!(
        requested_kinds(&mut queue).is_empty(),
        "nothing was advertised for `)`, so nothing is asked"
    );
    assert!(
        app.signature.is_none(),
        "a popup must never be left open over a call that has ended"
    );
}

#[test]
fn a_method_not_found_from_an_advertised_capability_is_still_a_retained_error() {
    let (mut app, _path, mut queue) = rust_app("fn f(a: i32) {}\nfn main() { f(\n");
    // `ready` advertises every capability, signature help included.
    ready(&mut app, Encoding::Utf8);
    drain(&mut queue);

    app.lsp_signature(SignatureContext::default());
    let token = drain(&mut queue)
        .into_iter()
        .find_map(|command| match command {
            LspCommand::Request { token, kind, .. }
                if matches!(*kind, RequestKind::SignatureHelp { .. }) =>
            {
                Some(token)
            }
            _ => None,
        })
        .expect("a request is sent when the server advertised the capability");

    app.apply_lsp_event(LspEvent::Response {
        token,
        response: Response::Failed("Method not found".to_owned()),
    });

    assert_eq!(app.unread_notification_counts().errors, 1);
    assert!(
        app.notifications.entries()[0]
            .body
            .contains("Method not found")
    );
}

#[test]
fn a_mismatched_language_response_is_retired_and_a_later_request_still_works() {
    let (mut app, _path, mut queue) = rust_app("fn main() {}\n");
    ready(&mut app, Encoding::Utf8);
    drain(&mut queue);

    app.lsp_hover();
    let first = drain(&mut queue)
        .into_iter()
        .find_map(|command| match command {
            LspCommand::Request { token, kind, .. } if matches!(*kind, RequestKind::Hover(_)) => {
                Some(token)
            }
            _ => None,
        })
        .expect("hover request");
    app.apply_lsp_event(LspEvent::Response {
        token: first,
        response: Response::Completions(Vec::new()),
    });

    assert!(app.status_error);
    assert!(
        app.status
            .contains("unexpected completion response for documentation")
    );
    assert!(!app.lsp_requests.contains_key(&first));
    assert!(app.hover.is_none());
    assert!(app.completion.is_none());

    app.lsp_hover();
    let second = drain(&mut queue)
        .into_iter()
        .find_map(|command| match command {
            LspCommand::Request { token, kind, .. } if matches!(*kind, RequestKind::Hover(_)) => {
                Some(token)
            }
            _ => None,
        })
        .expect("later hover request");
    assert_ne!(second, first);
    app.apply_lsp_event(LspEvent::Response {
        token: second,
        response: Response::Hover("usable documentation".to_owned()),
    });

    assert_eq!(
        app.hover.as_ref().map(|hover| hover.lines.clone()),
        Some(vec!["usable documentation".to_owned()])
    );
    assert!(!app.lsp_requests.contains_key(&second));
}

#[test]
fn a_single_goto_result_moves_the_caret_and_several_open_a_picker() {
    let (mut app, path, _queue) = rust_app("one\ntwo\nthree\n");
    ready(&mut app, Encoding::Utf8);

    app.lsp_requests.insert(
        99,
        tracked(
            &app,
            PendingRequest::Goto {
                label: "definition",
            },
        ),
    );
    app.apply_lsp_event(LspEvent::Response {
        token: 99,
        response: Response::Locations(vec![crate::lsp::Location {
            path: path.clone(),
            range: LspRange::new(LspPosition::new(1, 1), LspPosition::new(1, 3)),
            encoding: Encoding::Utf8,
        }]),
    });
    assert!(app.list.is_none());
    // The target range is selected, with the caret on its last character.
    let selection = app.active().selection.primary();
    assert_eq!(
        app.active_buffer().position_of(selection.from()),
        Position::new(1, 1)
    );
    assert_eq!(cursor(&app), Position::new(1, 2));

    app.lsp_requests.insert(
        100,
        tracked(
            &app,
            PendingRequest::Goto {
                label: "references",
            },
        ),
    );
    app.apply_lsp_event(LspEvent::Response {
        token: 100,
        response: Response::Locations(vec![
            crate::lsp::Location {
                path: path.clone(),
                range: LspRange::new(LspPosition::new(0, 0), LspPosition::new(0, 3)),
                encoding: Encoding::Utf8,
            },
            crate::lsp::Location {
                path: path.clone(),
                range: LspRange::new(LspPosition::new(2, 0), LspPosition::new(2, 5)),
                encoding: Encoding::Utf8,
            },
        ]),
    });
    let picker = app.list.as_ref().expect("several results open a picker");
    assert_eq!(picker.items.len(), 2);

    // Enter jumps to the selected row.
    key(&mut app, KeyCode::Down, Modifiers::NONE);
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert!(app.list.is_none());
    assert_eq!(cursor(&app).row, 2);
}

#[test]
fn a_cross_language_location_uses_the_sending_servers_encoding() {
    let (mut app, _source, _queue) = rust_app("fn main() {}\n");
    ready(&mut app, Encoding::Utf8);
    let target = temporary("target.py");
    fs::write(&target, "aéx\n").unwrap();
    app.lsp_requests.insert(
        101,
        tracked(
            &app,
            PendingRequest::Goto {
                label: "definition",
            },
        ),
    );
    app.apply_lsp_event(LspEvent::Response {
        token: 101,
        response: Response::Locations(vec![crate::lsp::Location {
            path: target.clone(),
            range: LspRange::new(LspPosition::new(0, 3), LspPosition::new(0, 4)),
            encoding: Encoding::Utf8,
        }]),
    });

    assert_eq!(app.active_buffer().path.as_deref(), Some(target.as_path()));
    assert_eq!(cursor(&app), Position::new(0, 2));
    fs::remove_file(target).unwrap();
}

#[test]
fn delayed_document_navigation_is_rejected_after_the_source_changes() {
    let (mut app, path, _queue) = rust_app("fn main() {}\n");
    ready(&mut app, Encoding::Utf8);
    for (token, pending, response) in [
        (
            102,
            PendingRequest::Goto {
                label: "definition",
            },
            Response::Locations(vec![crate::lsp::Location {
                path: path.clone(),
                range: LspRange::default(),
                encoding: Encoding::Utf8,
            }]),
        ),
        (
            103,
            PendingRequest::Symbols {
                title: "Document symbols",
                path: path.clone(),
            },
            Response::Symbols(Vec::new()),
        ),
    ] {
        let revision = app.buffers[0].revision();
        app.lsp_requests
            .insert(token, TrackedRequest::new(0, revision, pending));
        assert!(app.apply_to_buffer(0, &Transaction::insert(0, "// changed\n")));
        app.apply_lsp_event(LspEvent::Response { token, response });
        assert!(app.status.contains("stale language-server response"));
    }
}

#[test]
fn buffer_switch_cancels_delayed_hover_and_signature_ui() {
    for signature in [false, true] {
        let (mut app, _path, mut queue) = rust_app("fn main() {}\n");
        ready(&mut app, Encoding::Utf8);
        drain(&mut queue);
        if signature {
            app.lsp_signature(SignatureContext::default());
        } else {
            app.lsp_hover();
        }
        let token = drain(&mut queue)
            .into_iter()
            .find_map(|command| match command {
                LspCommand::Request { token, .. } => Some(token),
                _ => None,
            })
            .unwrap();
        app.buffers.push(Buffer::scratch());
        app.syntax.push(None);
        app.switch_buffer(1);
        assert!(drain(&mut queue).iter().any(
            |command| matches!(command, LspCommand::Cancel { token: cancelled } if *cancelled == token)
        ));
        app.apply_lsp_event(LspEvent::Response {
            token,
            response: if signature {
                Response::Signatures(vec![SignatureLine {
                    label: "fn()".to_owned(),
                    documentation: String::new(),
                    active_parameter: None,
                }])
            } else {
                Response::Hover("docs".to_owned())
            },
        });
        assert!(app.hover.is_none());
        assert!(app.signature.is_none());
    }
}

#[test]
fn a_rename_across_files_is_one_undo_step_per_file() {
    let (mut app, path, _queue) = rust_app("let name = 1;\nlet other = name;\n");
    ready(&mut app, Encoding::Utf8);
    let second = temporary("b.rs");

    app.lsp_requests.insert(
        5,
        tracked(
            &app,
            PendingRequest::Edits {
                label: "renamed",
                path: path.clone(),
            },
        ),
    );
    app.apply_lsp_event(LspEvent::Response {
        token: 5,
        response: Response::Edits {
            edits: vec![
                DocumentEdit {
                    path: path.clone(),
                    version: None,
                    edits: vec![edit(0, 4, 8, "renamed"), edit(1, 12, 16, "renamed")],
                },
                DocumentEdit {
                    path: second.clone(),
                    version: None,
                    edits: vec![edit(0, 0, 0, "// touched\n")],
                },
            ],
            skipped: 0,
            encoding: Encoding::Utf8,
        },
    });

    assert_eq!(text(&app), "let renamed = 1;\nlet other = renamed;\n");
    assert!(app.status.contains("2 files"), "{}", app.status);
    // A multi-range rename reverts in one step, and the second file was
    // opened as a buffer rather than written behind the person's back.
    press(&mut app, 'u');
    assert_eq!(text(&app), "let name = 1;\nlet other = name;\n");
    assert!(
        app.buffers
            .iter()
            .any(|buffer| buffer.path.as_deref() == Some(second.as_path()))
    );
    assert!(!second.exists(), "editing must not write to disk");
}

/// A language server names the files it wants changed, and nothing in the
/// protocol keeps those names inside the project.
#[test]
fn a_server_edit_outside_the_project_is_refused() {
    let (mut app, path, _queue) = rust_app("let name = 1;\n");
    ready(&mut app, Encoding::Utf8);
    let project = temporary("confined-project");
    fs::create_dir_all(&project).unwrap();
    app.project_root = project.clone();
    let inside = project.join("inside.rs");
    fs::write(&inside, "fn main() {}\n").unwrap();
    let outside = temporary("outside-bashrc");
    fs::write(&outside, "# untouched\n").unwrap();

    app.apply_lsp_event(LspEvent::ApplyEdit {
        language: "rust".into(),
        generation: 1,
        encoding: Encoding::Utf8,
        id: serde_json::json!(9),
        edits: vec![DocumentEdit {
            path: outside.clone(),
            version: None,
            edits: vec![edit(0, 0, 0, "malicious\n")],
        }],
        skipped: 0,
    });

    assert!(app.status_error, "{}", app.status);
    assert!(app.status.contains("refused"), "{}", app.status);
    assert!(
        !app.buffers
            .iter()
            .any(|buffer| buffer.path.as_deref() == Some(outside.as_path())),
        "the refused file was still opened as a buffer"
    );
    assert_eq!(fs::read_to_string(&outside).unwrap(), "# untouched\n");

    // A file inside the project is still edited normally.
    app.apply_lsp_event(LspEvent::ApplyEdit {
        language: "rust".into(),
        generation: 1,
        encoding: Encoding::Utf8,
        id: serde_json::json!(10),
        edits: vec![DocumentEdit {
            path: inside.clone(),
            version: None,
            edits: vec![edit(0, 0, 0, "// ok\n")],
        }],
        skipped: 0,
    });

    assert!(!app.status_error, "{}", app.status);
    let buffer = app
        .buffers
        .iter()
        .position(|buffer| buffer.path.as_deref() == Some(inside.as_path()))
        .expect("the in-project file was opened");
    assert!(app.buffers[buffer].to_string().starts_with("// ok"));

    let _ = fs::remove_file(&outside);
    let _ = fs::remove_dir_all(&project);
    let _ = fs::remove_file(&path);
}

#[test]
fn skipped_file_operations_are_reported_rather_than_performed() {
    let (mut app, path, _queue) = rust_app("x\n");
    ready(&mut app, Encoding::Utf8);
    app.lsp_requests.insert(
        6,
        tracked(
            &app,
            PendingRequest::Edits {
                label: "applied",
                path: path.clone(),
            },
        ),
    );
    app.apply_lsp_event(LspEvent::Response {
        token: 6,
        response: Response::Edits {
            edits: vec![DocumentEdit {
                path,
                version: None,
                edits: vec![edit(0, 0, 1, "y")],
            }],
            skipped: 2,
            encoding: Encoding::Utf8,
        },
    });
    assert!(
        app.status.contains("2 file operations not performed"),
        "{}",
        app.status
    );
}

#[test]
fn a_formatting_response_lands_on_the_document_that_was_asked_about() {
    let (mut app, path, _queue) = rust_app("fn  main(){}\n");
    ready(&mut app, Encoding::Utf8);
    app.lsp_requests.insert(
        8,
        tracked(
            &app,
            PendingRequest::Edits {
                label: "formatted",
                path: path.clone(),
            },
        ),
    );
    // Formatting results carry no URI, so the pending request supplies it.
    app.apply_lsp_event(LspEvent::Response {
        token: 8,
        response: Response::Edits {
            edits: vec![DocumentEdit {
                path: PathBuf::new(),
                version: None,
                edits: vec![edit(0, 0, 12, "fn main() {}")],
            }],
            skipped: 0,
            encoding: Encoding::Utf8,
        },
    });
    assert_eq!(text(&app), "fn main() {}\n");
}

#[test]
fn delayed_format_and_rename_responses_do_not_edit_a_newer_buffer_revision() {
    for (token, label) in [(81, "formatted"), (82, "renamed")] {
        let (mut app, path, _queue) = rust_app("fn  main(){}\n");
        ready(&mut app, Encoding::Utf8);
        let file = app.active().buffer;
        app.lsp_requests.insert(
            token,
            TrackedRequest::new(
                file,
                app.buffers[file].revision(),
                PendingRequest::Edits {
                    label,
                    path: path.clone(),
                },
            ),
        );

        assert!(app.edit(Transaction::insert(0, "// local\n")));
        let newer = text(&app);
        app.apply_lsp_event(LspEvent::Response {
            token,
            response: Response::Edits {
                edits: vec![DocumentEdit {
                    path: PathBuf::new(),
                    version: None,
                    edits: vec![edit(0, 0, 12, "fn main() {}")],
                }],
                skipped: 0,
                encoding: Encoding::Utf8,
            },
        });

        assert_eq!(text(&app), newer);
        assert!(app.status_error);
        assert!(app.status.contains("stale language-server response"));
    }
}

#[test]
fn a_multi_document_response_is_atomic_when_another_open_target_changed() {
    let (mut app, path, mut queue) = rust_app("let source = 1;\n");
    ready(&mut app, Encoding::Utf8);
    drain(&mut queue);
    let second = temporary("guarded-target.rs");
    fs::write(&second, "let target = 2;\n").unwrap();
    app.open_file(second.clone()).unwrap();
    let target = app.active().buffer;
    drain(&mut queue);
    app.switch_buffer(0);

    let guards = app.lsp_document_guards();
    app.lsp_requests.insert(
        811,
        TrackedRequest::new(
            0,
            app.buffers[0].revision(),
            PendingRequest::Edits {
                label: "renamed",
                path: path.clone(),
            },
        )
        .with_documents(guards),
    );
    assert!(app.apply_to_buffer(target, &Transaction::insert(0, "// local\n")));
    let source_before = app.buffers[0].to_string();
    let target_before = app.buffers[target].to_string();

    app.apply_lsp_event(LspEvent::Response {
        token: 811,
        response: Response::Edits {
            edits: vec![
                DocumentEdit {
                    path,
                    version: None,
                    edits: vec![edit(0, 4, 10, "changed")],
                },
                DocumentEdit {
                    path: second.clone(),
                    version: None,
                    edits: vec![edit(0, 4, 10, "changed")],
                },
            ],
            skipped: 0,
            encoding: Encoding::Utf8,
        },
    });

    assert_eq!(app.buffers[0].to_string(), source_before);
    assert_eq!(app.buffers[target].to_string(), target_before);
    assert!(app.status.contains("another document changed"));
    fs::remove_file(second).unwrap();
}

#[test]
fn a_multi_document_response_is_atomic_when_another_target_closed_or_moved() {
    for moved in [false, true] {
        let (mut app, path, mut queue) = rust_app("let source = 1;\n");
        ready(&mut app, Encoding::Utf8);
        drain(&mut queue);
        let second = temporary(if moved {
            "moved-target.rs"
        } else {
            "closed-target.rs"
        });
        fs::write(&second, "let target = 2;\n").unwrap();
        app.open_file(second.clone()).unwrap();
        let target = app.active().buffer;
        drain(&mut queue);
        app.switch_buffer(0);
        let guards = app.lsp_document_guards();
        app.lsp_requests.insert(
            812,
            TrackedRequest::new(
                0,
                app.buffers[0].revision(),
                PendingRequest::Edits {
                    label: "renamed",
                    path: path.clone(),
                },
            )
            .with_documents(guards),
        );
        if moved {
            app.buffers[target].path = Some(second.with_file_name("renamed.rs"));
        } else {
            app.close_buffer(target);
        }
        let before = app.buffers[0].to_string();

        app.apply_lsp_event(LspEvent::Response {
            token: 812,
            response: Response::Edits {
                edits: vec![DocumentEdit {
                    path,
                    version: None,
                    edits: vec![edit(0, 4, 10, "changed")],
                }],
                skipped: 0,
                encoding: Encoding::Utf8,
            },
        });

        assert_eq!(app.buffers[0].to_string(), before);
        assert!(app.status.contains("another document changed"));
        fs::remove_file(second).unwrap();
    }
}

#[test]
fn delayed_code_actions_are_not_offered_for_a_newer_buffer_revision() {
    let (mut app, _path, _queue) = rust_app("fn main() {}\n");
    ready(&mut app, Encoding::Utf8);
    let file = app.active().buffer;
    app.lsp_requests.insert(
        83,
        TrackedRequest::new(
            file,
            app.buffers[file].revision(),
            PendingRequest::CodeActions,
        ),
    );

    assert!(app.edit(Transaction::insert(0, "// local\n")));
    app.apply_lsp_event(LspEvent::Response {
        token: 83,
        response: Response::Actions(Vec::new()),
    });

    assert!(app.list.is_none());
    assert!(app.status_error);
    assert!(app.status.contains("stale language-server response"));
}

#[test]
fn server_lifecycle_does_not_close_a_picker_that_replaced_code_actions() {
    let (mut app, _path, _queue) = rust_app("fn main() {}\n");
    ready(&mut app, Encoding::Utf8);
    app.open_action_picker(
        vec![ActionEntry::unresolved_for_test("resolve")],
        0,
        app.buffers[0].revision(),
        app.lsp_document_guards(),
    );
    app.open_buffer_picker();
    let title = app.list.as_ref().unwrap().title.clone();

    app.apply_lsp_event(LspEvent::Restarted {
        language: "rust".to_owned(),
    });

    assert_eq!(
        app.list.as_ref().map(|list| list.title.as_str()),
        Some(title.as_str())
    );
    assert!(app.lsp_action_source.is_none());
}

#[test]
fn tab_requests_code_actions_in_an_ordinary_language_buffer() {
    let (mut app, _path, mut queue) = rust_app("fn main() {}\n");
    ready(&mut app, Encoding::Utf8);
    drain(&mut queue);

    key(&mut app, KeyCode::Tab, Modifiers::NONE);

    let commands = drain(&mut queue);
    assert!(
        commands.iter().any(|command| matches!(
            command,
            LspCommand::Request { kind, .. }
                if matches!(kind.as_ref(), RequestKind::CodeActions { .. })
        )),
        "{commands:?}"
    );
    assert!(app.context_action_menu.is_none());
}

#[test]
fn resolved_code_action_keeps_its_source_across_an_active_buffer_switch() {
    let (mut app, path, mut queue) = rust_app("fn main() {}\n");
    ready(&mut app, Encoding::Utf8);
    drain(&mut queue);
    let source = app.active().buffer;
    let source_revision = app.buffers[source].revision();
    let documents = app.lsp_document_guards();
    app.open_action_picker(
        vec![ActionEntry::unresolved_for_test("resolve me")],
        source,
        source_revision,
        documents,
    );
    app.buffers.push(Buffer::scratch());
    app.syntax.push(None);
    let scratch = app.buffers.len() - 1;
    app.switch_buffer(scratch);

    app.run_code_action(0);
    let token = drain(&mut queue)
        .into_iter()
        .find_map(|command| match command {
            LspCommand::Request {
                token,
                path: requested,
                ..
            } => {
                assert_eq!(requested, path);
                Some(token)
            }
            _ => None,
        })
        .expect("resolving the action queues a request");
    let tracked = app.lsp_requests.get(&token).expect("request is tracked");
    assert_eq!(tracked.buffer, source);
    assert_eq!(tracked.revision, source_revision);

    assert!(app.apply_to_buffer(source, &Transaction::insert(0, "// local\n")));
    let newer = app.buffers[source].to_string();
    app.apply_lsp_event(LspEvent::Response {
        token,
        response: Response::Edits {
            edits: vec![DocumentEdit {
                path,
                version: None,
                edits: vec![edit(0, 0, 2, "// server")],
            }],
            skipped: 0,
            encoding: Encoding::Utf8,
        },
    });

    assert_eq!(app.buffers[source].to_string(), newer);
    assert!(app.status_error);
    assert!(app.status.contains("stale language-server response"));
}

#[test]
fn resolved_code_action_command_keeps_server_provenance_and_is_preflighted() {
    for (command_name, skipped, accepted) in [
        ("mock.command", 0, true),
        ("not.advertised", 0, false),
        ("mock.command", 1, false),
    ] {
        let (mut app, path, mut queue) = rust_app("original\n");
        ready(&mut app, Encoding::Utf8);
        drain(&mut queue);
        let documents = app.lsp_document_guards();
        app.lsp_requests.insert(
            104,
            TrackedRequest::new(
                0,
                app.buffers[0].revision(),
                PendingRequest::Edits {
                    label: "applied",
                    path: PathBuf::new(),
                },
            )
            .with_documents(documents)
            .with_server("rust".to_owned(), 1),
        );
        app.apply_lsp_event(LspEvent::Response {
            token: 104,
            response: Response::ActionEdits {
                edits: vec![DocumentEdit {
                    path,
                    version: None,
                    edits: vec![edit(0, 0, 8, "changed")],
                }],
                skipped,
                encoding: Encoding::Utf8,
                command: Some(lsp_types::Command {
                    title: "finish".to_owned(),
                    command: command_name.to_owned(),
                    arguments: None,
                }),
            },
        });

        if accepted {
            assert_eq!(text(&app), "changed\n");
            assert!(drain(&mut queue).iter().any(|command| matches!(
                command,
                LspCommand::Request { language, kind, .. }
                    if language == "rust"
                        && matches!(kind.as_ref(), RequestKind::ExecuteCommand(command) if command.command == "mock.command")
            )));
        } else {
            assert_eq!(text(&app), "original\n");
            assert!(drain(&mut queue).is_empty());
        }
    }
}

#[test]
fn code_action_command_is_suppressed_when_an_edit_cannot_be_synchronized() {
    let (mut app, path, mut queue) = rust_app("original\n");
    ready(&mut app, Encoding::Utf8);
    drain(&mut queue);
    let documents = app.lsp_document_guards();
    app.lsp_requests.insert(
        105,
        TrackedRequest::new(
            0,
            app.buffers[0].revision(),
            PendingRequest::Edits {
                label: "applied",
                path: PathBuf::new(),
            },
        )
        .with_documents(documents)
        .with_server("rust".to_owned(), 1),
    );
    for _ in 0..crate::lsp::COMMAND_CAPACITY {
        assert!(app.lsp_send(LspCommand::Status));
    }

    app.apply_lsp_event(LspEvent::Response {
        token: 105,
        response: Response::ActionEdits {
            edits: vec![DocumentEdit {
                path,
                version: None,
                edits: vec![edit(0, 0, 8, "changed")],
            }],
            skipped: 0,
            encoding: Encoding::Utf8,
            command: Some(lsp_types::Command {
                title: "finish".to_owned(),
                command: "mock.command".to_owned(),
                arguments: None,
            }),
        },
    });

    assert_eq!(text(&app), "changed\n");
    assert!(app.lsp_documents.get(&0).unwrap().desynced);
    assert!(app.status_error);
    assert!(app.status.contains("command not sent"));
    assert!(drain(&mut queue).iter().all(|command| {
        !matches!(
            command,
            LspCommand::Request { kind, .. }
                if matches!(kind.as_ref(), RequestKind::ExecuteCommand(_))
        )
    }));
}

#[test]
fn code_action_command_is_suppressed_when_a_new_target_cannot_be_opened() {
    let (mut app, _path, mut queue) = rust_app("original\n");
    ready(&mut app, Encoding::Utf8);
    drain(&mut queue);
    let target = temporary("new-target.rs");
    fs::write(&target, "before\n").unwrap();
    let documents = app.lsp_document_guards();
    app.lsp_requests.insert(
        106,
        TrackedRequest::new(
            0,
            app.buffers[0].revision(),
            PendingRequest::Edits {
                label: "applied",
                path: PathBuf::new(),
            },
        )
        .with_documents(documents)
        .with_server("rust".to_owned(), 1),
    );
    for _ in 0..crate::lsp::COMMAND_CAPACITY {
        assert!(app.lsp_send(LspCommand::Status));
    }

    app.apply_lsp_event(LspEvent::Response {
        token: 106,
        response: Response::ActionEdits {
            edits: vec![DocumentEdit {
                path: target.clone(),
                version: None,
                edits: vec![edit(0, 0, 6, "changed")],
            }],
            skipped: 0,
            encoding: Encoding::Utf8,
            command: Some(lsp_types::Command {
                title: "finish".to_owned(),
                command: "mock.command".to_owned(),
                arguments: None,
            }),
        },
    });

    let target_buffer = app
        .buffers
        .iter()
        .position(|buffer| buffer.path.as_deref() == Some(target.as_path()))
        .unwrap();
    assert_eq!(app.buffers[target_buffer].to_string(), "changed\n");
    assert!(!app.lsp_documents.contains_key(&target_buffer));
    assert!(app.status.contains("command not sent"));
    assert!(drain(&mut queue).iter().all(|command| {
        !matches!(
            command,
            LspCommand::Request { kind, .. }
                if matches!(kind.as_ref(), RequestKind::ExecuteCommand(_))
        )
    }));
    fs::remove_file(target).unwrap();
}

#[test]
fn code_action_command_is_suppressed_for_a_target_owned_by_another_server() {
    let (mut app, _path, mut queue) = rust_app("original\n");
    ready(&mut app, Encoding::Utf8);
    drain(&mut queue);
    let target = temporary("other.py");
    fs::write(&target, "before\n").unwrap();
    app.open_file(target.clone()).unwrap();
    ready_language(&mut app, "python", Encoding::Utf8);
    drain(&mut queue);
    let documents = app.lsp_document_guards();
    app.lsp_requests.insert(
        107,
        TrackedRequest::new(
            0,
            app.buffers[0].revision(),
            PendingRequest::Edits {
                label: "applied",
                path: PathBuf::new(),
            },
        )
        .with_documents(documents)
        .with_server("rust".to_owned(), 1),
    );

    app.apply_lsp_event(LspEvent::Response {
        token: 107,
        response: Response::ActionEdits {
            edits: vec![DocumentEdit {
                path: target.clone(),
                version: None,
                edits: vec![edit(0, 0, 6, "changed")],
            }],
            skipped: 0,
            encoding: Encoding::Utf8,
            command: Some(lsp_types::Command {
                title: "finish".to_owned(),
                command: "mock.command".to_owned(),
                arguments: None,
            }),
        },
    });

    assert_eq!(text(&app), "changed\n");
    assert!(app.status.contains("command not sent"));
    let commands = drain(&mut queue);
    assert!(commands.iter().any(
        |command| matches!(command, LspCommand::Change { language, .. } if language == "python")
    ));
    assert!(commands.iter().all(|command| {
        !matches!(
            command,
            LspCommand::Request { kind, .. }
                if matches!(kind.as_ref(), RequestKind::ExecuteCommand(_))
        )
    }));
    fs::remove_file(target).unwrap();
}

#[test]
fn workspace_edit_acknowledgement_retries_after_manager_backpressure() {
    let (mut app, path, mut queue) = rust_app("original\n");
    ready(&mut app, Encoding::Utf8);
    drain(&mut queue);
    for _ in 0..crate::lsp::COMMAND_CAPACITY {
        assert!(app.lsp_send(LspCommand::Status));
    }

    app.apply_lsp_event(LspEvent::ApplyEdit {
        language: "rust".into(),
        generation: 1,
        encoding: Encoding::Utf8,
        id: serde_json::json!(900),
        edits: vec![DocumentEdit {
            path,
            version: None,
            edits: vec![edit(0, 0, 8, "changed")],
        }],
        skipped: 0,
    });
    assert_eq!(text(&app), "changed\n");
    assert_eq!(app.pending_lsp_replies.len(), 1);

    drain(&mut queue);
    app.flush_lsp_replies();
    assert!(matches!(
        drain(&mut queue).as_slice(),
        [LspCommand::EditApplied { id, applied: true, .. }]
            if id == &serde_json::json!(900)
    ));
    assert!(app.pending_lsp_replies.is_empty());
}

#[test]
fn versioned_workspace_edit_is_rejected_after_the_document_advances() {
    let (mut app, path, mut queue) = rust_app("fn main() {}\n");
    ready(&mut app, Encoding::Utf8);
    drain(&mut queue);

    assert!(app.edit(Transaction::insert(0, "// local\n")));
    let newer = text(&app);
    drain(&mut queue);
    app.apply_lsp_event(LspEvent::ApplyEdit {
        language: "rust".into(),
        generation: 1,
        encoding: Encoding::Utf8,
        id: serde_json::json!(84),
        edits: vec![DocumentEdit {
            path,
            version: Some(1),
            edits: vec![edit(0, 0, 2, "// server")],
        }],
        skipped: 0,
    });

    assert_eq!(text(&app), newer);
    assert!(app.status_error);
    assert!(app.status.contains("language-server version 1"));
    assert!(matches!(
        drain(&mut queue).as_slice(),
        [LspCommand::EditApplied { applied: false, .. }]
    ));
}

#[test]
fn a_later_invalid_document_rejects_a_workspace_edit_atomically() {
    let (mut app, path, mut queue) = rust_app("let original = 1;\n");
    ready(&mut app, Encoding::Utf8);
    drain(&mut queue);
    let binary = temporary("binary.rs");
    fs::write(&binary, [0xff, 0xfe]).unwrap();
    let before = text(&app);

    app.apply_lsp_event(LspEvent::ApplyEdit {
        language: "rust".into(),
        generation: 1,
        encoding: Encoding::Utf8,
        id: serde_json::json!(841),
        edits: vec![
            DocumentEdit {
                path,
                version: None,
                edits: vec![edit(0, 4, 12, "changed")],
            },
            DocumentEdit {
                path: binary.clone(),
                version: None,
                edits: vec![edit(0, 0, 0, "text")],
            },
        ],
        skipped: 0,
    });

    assert_eq!(text(&app), before);
    assert!(app.status_error);
    assert!(
        app.buffers
            .iter()
            .all(|buffer| { buffer.path.as_deref() != Some(binary.as_path()) })
    );
    assert!(matches!(
        drain(&mut queue).as_slice(),
        [LspCommand::EditApplied { applied: false, .. }]
    ));
    fs::remove_file(binary).unwrap();
}

#[test]
fn duplicate_workspace_edit_documents_form_one_transaction() {
    let (mut app, path, mut queue) = rust_app("one two\n");
    ready(&mut app, Encoding::Utf8);
    drain(&mut queue);

    app.apply_lsp_event(LspEvent::ApplyEdit {
        language: "rust".into(),
        generation: 1,
        encoding: Encoding::Utf8,
        id: serde_json::json!(842),
        edits: vec![
            DocumentEdit {
                path: path.clone(),
                version: None,
                edits: vec![edit(0, 0, 3, "1")],
            },
            DocumentEdit {
                path,
                version: None,
                edits: vec![edit(0, 4, 7, "2")],
            },
        ],
        skipped: 0,
    });

    assert_eq!(text(&app), "1 2\n");
    press(&mut app, 'u');
    assert_eq!(text(&app), "one two\n");
}

#[test]
fn overlapping_duplicate_workspace_edits_are_rejected_atomically() {
    let (mut app, path, mut queue) = rust_app("original\n");
    ready(&mut app, Encoding::Utf8);
    drain(&mut queue);

    app.apply_lsp_event(LspEvent::ApplyEdit {
        language: "rust".into(),
        generation: 1,
        encoding: Encoding::Utf8,
        id: serde_json::json!(843),
        edits: vec![
            DocumentEdit {
                path: path.clone(),
                version: None,
                edits: vec![edit(0, 0, 8, "first")],
            },
            DocumentEdit {
                path,
                version: None,
                edits: vec![edit(0, 0, 8, "second")],
            },
        ],
        skipped: 0,
    });

    assert_eq!(text(&app), "original\n");
    assert!(app.status_error);
    assert!(app.status.contains("overlapping"));
    assert!(matches!(
        drain(&mut queue).as_slice(),
        [LspCommand::EditApplied { applied: false, .. }]
    ));
}

#[test]
fn malformed_workspace_edit_ranges_are_rejected_atomically() {
    for invalid in [
        edit(9, 0, 0, "outside"),
        crate::lsp::TextEdit {
            range: LspRange::new(LspPosition::new(0, 8), LspPosition::new(0, 1)),
            new_text: "reversed".to_owned(),
        },
    ] {
        let (mut app, path, mut queue) = rust_app("original\n");
        ready(&mut app, Encoding::Utf8);
        drain(&mut queue);
        app.apply_lsp_event(LspEvent::ApplyEdit {
            language: "rust".into(),
            generation: 1,
            encoding: Encoding::Utf8,
            id: serde_json::json!(845),
            edits: vec![DocumentEdit {
                path,
                version: None,
                edits: vec![invalid],
            }],
            skipped: 0,
        });
        assert_eq!(text(&app), "original\n");
        assert!(app.status.contains("invalid language-server edit range"));
        assert!(matches!(
            drain(&mut queue).as_slice(),
            [LspCommand::EditApplied { applied: false, .. }]
        ));
    }
}

#[test]
fn a_workspace_edit_cannot_split_an_encoded_character() {
    let (mut app, path, mut queue) = rust_app("a🦀b\n");
    ready(&mut app, Encoding::Utf8);
    drain(&mut queue);
    app.apply_lsp_event(LspEvent::ApplyEdit {
        language: "rust".into(),
        generation: 1,
        encoding: Encoding::Utf8,
        id: serde_json::json!(846),
        edits: vec![DocumentEdit {
            path,
            version: None,
            edits: vec![edit(0, 2, 2, "x")],
        }],
        skipped: 0,
    });
    assert_eq!(text(&app), "a🦀b\n");
    assert!(app.status.contains("invalid language-server edit range"));
}

#[cfg(unix)]
#[test]
fn a_versioned_workspace_edit_reuses_an_open_symlink_alias() {
    use std::os::unix::fs::symlink;

    let root = temporary("symlink-edit-root");
    fs::create_dir_all(&root).unwrap();
    let target = root.join("target.rs");
    let link = root.join("link.rs");
    fs::write(&target, "original\n").unwrap();
    symlink(&target, &link).unwrap();
    let (mut app, _old_path, mut queue) = rust_app("original\n");
    app.project_root = root.clone();
    app.buffers[0].path = Some(link.clone());
    ready(&mut app, Encoding::Utf8);
    drain(&mut queue);
    let version = app.lsp_documents[&0].version;

    app.apply_lsp_event(LspEvent::ApplyEdit {
        language: "rust".into(),
        generation: 1,
        encoding: Encoding::Utf8,
        id: serde_json::json!(844),
        edits: vec![DocumentEdit {
            path: link,
            version: Some(version),
            edits: vec![edit(0, 0, 8, "changed")],
        }],
        skipped: 0,
    });

    assert_eq!(text(&app), "changed\n");
    assert_eq!(app.buffers.len(), 1, "the target must not open twice");
    assert!(!app.status_error, "{}", app.status);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn nonexistent_workspace_edit_aliases_have_one_identity() {
    let root = temporary("workspace-edit-identity");
    fs::create_dir_all(&root).unwrap();
    let direct = workspace_edit_path_identity(&root.join("new.rs")).unwrap();
    let alias = workspace_edit_path_identity(&root.join("missing/../new.rs")).unwrap();
    assert_eq!(direct, alias);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn numeric_workspace_version_cannot_be_satisfied_by_opening_a_closed_file() {
    let (mut app, path, mut queue) = rust_app("fn main() {}\n");
    ready(&mut app, Encoding::Utf8);
    drain(&mut queue);
    let closed = path.with_file_name("closed.rs");
    fs::write(&closed, "fn closed() {}\n").unwrap();

    app.apply_lsp_event(LspEvent::ApplyEdit {
        language: "rust".into(),
        generation: 1,
        encoding: Encoding::Utf8,
        id: serde_json::json!(85),
        edits: vec![DocumentEdit {
            path: closed.clone(),
            version: Some(1),
            edits: vec![edit(0, 0, 2, "// server")],
        }],
        skipped: 0,
    });

    assert!(app.status_error);
    assert!(app.status.contains("language-server version 1"));
    assert!(
        app.buffers
            .iter()
            .all(|buffer| { buffer.path.as_deref() != Some(closed.as_path()) })
    );
    assert_eq!(fs::read_to_string(&closed).unwrap(), "fn closed() {}\n");
    assert!(matches!(
        drain(&mut queue).as_slice(),
        [LspCommand::EditApplied { applied: false, .. }]
    ));
    fs::remove_file(closed).unwrap();
}

#[test]
fn closing_a_buffer_ignores_its_delayed_edit_response() {
    let (mut app, path, _queue) = rust_app("fn  main(){}\n");
    ready(&mut app, Encoding::Utf8);
    let file = app.active().buffer;
    app.lsp_requests.insert(
        8,
        TrackedRequest::new(
            file,
            app.buffers[file].revision(),
            PendingRequest::Edits {
                label: "formatted",
                path: path.clone(),
            },
        ),
    );

    app.close_buffer(file);
    let buffers_after_close = app.buffers.len();
    assert!(!app.lsp_requests.contains_key(&8));

    app.apply_lsp_event(LspEvent::Response {
        token: 8,
        response: Response::Edits {
            edits: vec![DocumentEdit {
                path: PathBuf::new(),
                version: None,
                edits: vec![edit(0, 0, 12, "fn main() {}")],
            }],
            skipped: 0,
            encoding: Encoding::Utf8,
        },
    });

    assert_eq!(app.buffers.len(), buffers_after_close);
    assert!(app.buffers.iter().enumerate().all(|(index, buffer)| {
        app.closed_buffers.contains(&index) || buffer.path.as_deref() != Some(path.as_path())
    }));
}

#[test]
fn completion_filters_while_typing_and_inserts_with_its_extra_edits() {
    let (mut app, _, mut queue) = rust_app("use std;\nfn main() { }\n");
    ready(&mut app, Encoding::Utf8);
    drain(&mut queue);

    set_cursor(&mut app, 1, 12);
    press(&mut app, 'i');
    let anchor = app.active().head();
    app.lsp_requests.insert(
        3,
        tracked(
            &app,
            PendingRequest::Completion {
                buffer: 0,
                anchor,
                explicit_session: None,
            },
        ),
    );
    app.apply_lsp_event(LspEvent::Response {
        token: 3,
        response: Response::Completions(vec![
            crate::lsp::Completion {
                label: "println".to_owned(),
                filter_text: None,
                sort_text: None,
                detail: "macro".to_owned(),
                kind: "function",
                insert: "println".to_owned(),
                edit: None,
                additional: vec![edit(0, 0, 0, "use std::fmt;\n")],
            },
            crate::lsp::Completion {
                label: "panic".to_owned(),
                filter_text: None,
                sort_text: None,
                detail: String::new(),
                kind: "function",
                insert: "panic".to_owned(),
                edit: None,
                additional: Vec::new(),
            },
        ]),
    });
    assert_eq!(app.completion.as_ref().unwrap().items.len(), 2);

    // Typing narrows the popup locally, with no further round trip.
    type_text(&mut app, "pr");
    let state = app.completion.as_ref().expect("popup should still be open");
    assert_eq!(state.visible_indices().len(), 1);
    assert_eq!(state.selected_item().unwrap().label, "println");

    key(&mut app, KeyCode::Tab, Modifiers::NONE);
    assert!(app.completion.is_none());
    assert_eq!(
        text(&app),
        "use std::fmt;\nuse std;\nfn main() { println}\n"
    );
    // The typed prefix, accepted word, and its import are one Insert
    // action and therefore revert together.
    key(&mut app, KeyCode::Escape, Modifiers::NONE);
    press(&mut app, 'u');
    assert_eq!(text(&app), "use std;\nfn main() { }\n");
}

#[test]
fn explicit_completion_filters_the_existing_prefix_and_replaces_it() {
    let (mut app, _, mut queue) = rust_app("left_at\n");
    ready(&mut app, Encoding::Utf8);
    drain(&mut queue);
    set_cursor(&mut app, 0, 7);
    press(&mut app, 'i');
    app.completion = Some(CompletionState {
        items: vec![crate::lsp::Completion {
            label: "left_atlas".to_owned(),
            filter_text: None,
            sort_text: None,
            detail: String::new(),
            kind: "word",
            insert: "left_atlas".to_owned(),
            edit: None,
            additional: Vec::new(),
        }],
        selected: 0,
        buffer: 0,
        anchor: 0,
        filter: "left_at".to_owned(),
        source: CompletionSource::Word,
        explicit_session: None,
    });

    key(&mut app, KeyCode::Char('x'), Modifiers::CONTROL);
    let pending = app.completion.as_ref().expect("Ctrl-x starts a session");
    assert_eq!(pending.source, CompletionSource::Language);
    assert!(
        pending.items.is_empty(),
        "the Word popup disappears at once"
    );
    assert_eq!(pending.anchor, 0);
    assert_eq!(pending.filter, "left_at");

    let token = drain(&mut queue)
        .into_iter()
        .find_map(|command| match command {
            LspCommand::Request { token, kind, .. }
                if matches!(*kind, RequestKind::Completion(_)) =>
            {
                Some(token)
            }
            _ => None,
        })
        .expect("Ctrl-x sends completion");
    app.apply_lsp_event(LspEvent::Response {
        token,
        response: Response::Completions(vec![
            crate::lsp::Completion {
                label: "self::".to_owned(),
                filter_text: None,
                sort_text: Some("001".to_owned()),
                detail: String::new(),
                kind: "keyword",
                insert: "self::".to_owned(),
                edit: None,
                additional: Vec::new(),
            },
            crate::lsp::Completion {
                label: "left_atomic".to_owned(),
                filter_text: None,
                sort_text: Some("999".to_owned()),
                detail: String::new(),
                kind: "variable",
                insert: "left_atomic".to_owned(),
                edit: None,
                additional: Vec::new(),
            },
        ]),
    });

    let state = app.completion.as_ref().unwrap();
    assert_eq!(state.filter, "left_at");
    assert_eq!(state.visible_indices().len(), 1);
    assert_eq!(state.selected_item().unwrap().label, "left_atomic");
    key(&mut app, KeyCode::Tab, Modifiers::NONE);
    assert_eq!(text(&app), "left_atomic\n");
}

#[test]
fn explicit_completion_stays_pinned_without_matches_and_rejects_late_responses() {
    let (mut app, _, mut queue) = rust_app("left\n");
    ready(&mut app, Encoding::Utf8);
    drain(&mut queue);
    set_cursor(&mut app, 0, 4);
    press(&mut app, 'i');
    key(&mut app, KeyCode::Char('x'), Modifiers::CONTROL);
    let token = drain(&mut queue)
        .into_iter()
        .find_map(|command| match command {
            LspCommand::Request { token, kind, .. }
                if matches!(*kind, RequestKind::Completion(_)) =>
            {
                Some(token)
            }
            _ => None,
        })
        .unwrap();
    app.apply_lsp_event(LspEvent::Response {
        token,
        response: Response::Completions(vec![crate::lsp::Completion {
            label: "leftward".to_owned(),
            filter_text: None,
            sort_text: None,
            detail: String::new(),
            kind: "variable",
            insert: "leftward".to_owned(),
            edit: None,
            additional: Vec::new(),
        }]),
    });

    press(&mut app, 'z');
    let state = app
        .completion
        .as_ref()
        .expect("the LSP session stays active");
    assert_eq!(state.source, CompletionSource::Language);
    assert!(state.explicit_session.is_some());
    assert!(state.visible_indices().is_empty());
    assert!(
        app.overlay_snapshots()
            .iter()
            .all(|overlay| { overlay.kind != crate::snapshot::OverlayKind::Completion }),
        "an empty session is not presented as a popup"
    );

    key(&mut app, KeyCode::Backspace, Modifiers::NONE);
    assert_eq!(
        app.completion
            .as_ref()
            .unwrap()
            .selected_item()
            .unwrap()
            .label,
        "leftward"
    );
    press(&mut app, '/');
    assert_eq!(
        app.completion.as_ref().unwrap().source,
        CompletionSource::Language,
        "Path completion cannot replace an explicit LSP session"
    );

    key(&mut app, KeyCode::Char('x'), Modifiers::CONTROL);
    let late = drain(&mut queue)
        .into_iter()
        .find_map(|command| match command {
            LspCommand::Request { token, kind, .. }
                if matches!(*kind, RequestKind::Completion(_)) =>
            {
                Some(token)
            }
            _ => None,
        })
        .unwrap();
    key(&mut app, KeyCode::Escape, Modifiers::NONE);
    assert_eq!(
        app.mode,
        Mode::Insert,
        "Escape dismisses before leaving Insert"
    );
    assert!(app.completion.is_none());
    app.apply_lsp_event(LspEvent::Response {
        token: late,
        response: Response::Completions(vec![crate::lsp::Completion {
            label: "late".to_owned(),
            filter_text: None,
            sort_text: None,
            detail: String::new(),
            kind: "variable",
            insert: "late".to_owned(),
            edit: None,
            additional: Vec::new(),
        }]),
    });
    assert!(app.completion.is_none());
}

#[test]
fn explicit_completion_refreshes_context_and_ends_at_editing_boundaries() {
    let (mut app, _, mut queue) = rust_app("left\n");
    ready(&mut app, Encoding::Utf8);
    drain(&mut queue);
    set_cursor(&mut app, 0, 4);
    press(&mut app, 'i');
    key(&mut app, KeyCode::Char('x'), Modifiers::CONTROL);
    let first = drain(&mut queue)
        .into_iter()
        .find_map(|command| match command {
            LspCommand::Request { token, kind, .. }
                if matches!(*kind, RequestKind::Completion(_)) =>
            {
                Some(token)
            }
            _ => None,
        })
        .unwrap();

    press(&mut app, '.');
    let second = drain(&mut queue)
        .into_iter()
        .find_map(|command| match command {
            LspCommand::Request { token, kind, .. }
                if matches!(*kind, RequestKind::Completion(_)) =>
            {
                Some(token)
            }
            _ => None,
        })
        .expect("a trigger character refreshes the explicit session");
    assert_ne!(first, second);
    app.apply_lsp_event(LspEvent::Response {
        token: first,
        response: Response::Completions(vec![crate::lsp::Completion {
            label: "stale".to_owned(),
            filter_text: None,
            sort_text: None,
            detail: String::new(),
            kind: "variable",
            insert: "stale".to_owned(),
            edit: None,
            additional: Vec::new(),
        }]),
    });
    assert!(app.completion.as_ref().unwrap().items.is_empty());
    app.apply_lsp_event(LspEvent::Response {
        token: second,
        response: Response::Completions(vec![crate::lsp::Completion {
            label: "field".to_owned(),
            filter_text: None,
            sort_text: None,
            detail: String::new(),
            kind: "field",
            insert: "field".to_owned(),
            edit: None,
            additional: Vec::new(),
        }]),
    });
    assert_eq!(
        app.completion
            .as_ref()
            .unwrap()
            .selected_item()
            .unwrap()
            .label,
        "field"
    );

    press(&mut app, ' ');
    assert!(app.completion.is_none());
    key(&mut app, KeyCode::Char('x'), Modifiers::CONTROL);
    drain(&mut queue);
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert!(app.completion.is_none());
    assert_eq!(text(&app), "left. \n\n");

    key(&mut app, KeyCode::Char('x'), Modifiers::CONTROL);
    drain(&mut queue);
    key(&mut app, KeyCode::Left, Modifiers::NONE);
    assert!(
        app.completion.is_none(),
        "caret movement cancels the session"
    );
}

#[test]
fn language_completion_uses_filter_text_and_sort_text() {
    let state = CompletionState {
        items: vec![
            crate::lsp::Completion {
                label: "shown_second".to_owned(),
                filter_text: Some("left_beta".to_owned()),
                sort_text: Some("002".to_owned()),
                detail: String::new(),
                kind: "variable",
                insert: String::new(),
                edit: None,
                additional: Vec::new(),
            },
            crate::lsp::Completion {
                label: "shown_first".to_owned(),
                filter_text: Some("left_alpha".to_owned()),
                sort_text: Some("001".to_owned()),
                detail: String::new(),
                kind: "variable",
                insert: String::new(),
                edit: None,
                additional: Vec::new(),
            },
            crate::lsp::Completion {
                label: "self::".to_owned(),
                filter_text: None,
                sort_text: Some("000".to_owned()),
                detail: String::new(),
                kind: "keyword",
                insert: String::new(),
                edit: None,
                additional: Vec::new(),
            },
        ],
        selected: 0,
        buffer: 0,
        anchor: 0,
        filter: "LEFT_".to_owned(),
        source: CompletionSource::Language,
        explicit_session: Some(1),
    };
    let labels: Vec<_> = state
        .visible_indices()
        .into_iter()
        .map(|index| state.items[index].label.as_str())
        .collect();
    assert_eq!(labels, vec!["shown_first", "shown_second"]);
}

fn word_completion_geometry() -> FrameGeometry {
    FrameGeometry {
        screen: Rect {
            width: 40,
            height: 22,
            ..Rect::default()
        },
        editor: Rect {
            width: 40,
            height: 20,
            ..Rect::default()
        },
        status: Rect::default(),
        message: Rect::default(),
    }
}

#[test]
fn word_completion_triggers_after_the_minimum_and_orders_own_buffer_first() {
    let root = temporary("word-completion-trigger");
    fs::create_dir_all(&root).unwrap();
    let active = root.join("a.txt");
    fs::write(&active, "wobble wobble\n").unwrap();
    let other = root.join("b.txt");
    fs::write(&other, "wobblegum\n").unwrap();
    let mut app = App::new_in_project(Config::default(), Some(active), &root).unwrap();
    let handle = crate::word_index::spawn();
    app.attach_word_index(handle.clone());
    app.host_open_file(other, false).unwrap();
    app.prepare_view(word_completion_geometry());
    handle.flush();

    set_cursor(&mut app, 1, 0);
    press(&mut app, 'i');
    type_text(&mut app, "wob");

    let state = app.completion.as_ref().expect("word popup should be open");
    assert_eq!(state.source, CompletionSource::Word);
    let labels: Vec<&str> = state
        .visible_indices()
        .into_iter()
        .map(|index| state.items[index].label.as_str())
        .collect();
    // "wobble" (own buffer, seen twice) precedes "wobblegum" (the other
    // buffer), and "wob" itself, the word being typed, is never offered.
    assert_eq!(labels, vec!["wobble", "wobblegum"]);

    // Tab accepts a word completion; Enter deliberately does not (see
    // `word_completion_lets_enter_insert_a_newline_instead_of_accepting`).
    key(&mut app, KeyCode::Tab, Modifiers::NONE);
    assert!(app.completion.is_none());
    assert_eq!(text(&app), "wobble wobble\nwobble");

    // The typed prefix and the accepted word are one Insert action.
    key(&mut app, KeyCode::Escape, Modifiers::NONE);
    press(&mut app, 'u');
    assert_eq!(text(&app), "wobble wobble\n");

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn word_completion_keeps_a_hyphen_that_joins_word_parts() {
    let root = temporary("word-completion-hyphenated-word");
    fs::create_dir_all(&root).unwrap();
    let active = root.join("a.txt");
    fs::write(&active, "up-to-date\n").unwrap();
    let mut app = App::new_in_project(Config::default(), Some(active), &root).unwrap();
    let handle = crate::word_index::spawn();
    app.attach_word_index(handle.clone());
    app.prepare_view(word_completion_geometry());
    handle.flush();

    set_cursor(&mut app, 1, 0);
    press(&mut app, 'i');
    type_text(&mut app, "up-");

    let state = app.completion.as_ref().expect("word popup should be open");
    assert_eq!(state.source, CompletionSource::Word);
    assert_eq!(state.selected_item().unwrap().label, "up-to-date");

    key(&mut app, KeyCode::Tab, Modifiers::NONE);
    assert_eq!(text(&app), "up-to-date\nup-to-date");

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn word_completion_lets_enter_insert_a_newline_instead_of_accepting() {
    let root = temporary("word-completion-enter-newline");
    fs::create_dir_all(&root).unwrap();
    let active = root.join("a.txt");
    fs::write(&active, "wobble wobble\n").unwrap();
    let mut app = App::new_in_project(Config::default(), Some(active), &root).unwrap();
    let handle = crate::word_index::spawn();
    app.attach_word_index(handle.clone());
    app.prepare_view(word_completion_geometry());
    handle.flush();

    set_cursor(&mut app, 1, 0);
    press(&mut app, 'i');
    type_text(&mut app, "wob");
    assert_eq!(
        app.completion.as_ref().unwrap().source,
        CompletionSource::Word
    );

    // A word popup opens on its own far more often than Language or Path
    // ever did, so Enter must keep meaning "newline" rather than risk
    // silently accepting a word nobody asked for while finishing a line.
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert!(app.completion.is_none());
    assert_eq!(text(&app), "wobble wobble\nwob\n");

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn word_completion_escape_dismisses_the_popup_and_leaves_insert_mode() {
    let root = temporary("word-completion-escape-normal");
    fs::create_dir_all(&root).unwrap();
    let active = root.join("a.txt");
    fs::write(&active, "wobble wobble\n").unwrap();
    let mut app = App::new_in_project(Config::default(), Some(active), &root).unwrap();
    let handle = crate::word_index::spawn();
    app.attach_word_index(handle.clone());
    app.prepare_view(word_completion_geometry());
    handle.flush();

    set_cursor(&mut app, 1, 0);
    press(&mut app, 'i');
    type_text(&mut app, "wob");
    assert_eq!(
        app.completion.as_ref().unwrap().source,
        CompletionSource::Word
    );

    key(&mut app, KeyCode::Escape, Modifiers::NONE);

    assert!(app.completion.is_none());
    assert_eq!(app.mode, Mode::Normal);

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn word_completion_is_replaced_by_a_language_response_but_never_opens_over_one() {
    let root = temporary("word-completion-language-precedence");
    fs::create_dir_all(&root).unwrap();
    let active = root.join("a.txt");
    fs::write(&active, "wobble wobble\n").unwrap();
    let mut app = App::new_in_project(Config::default(), Some(active), &root).unwrap();
    let handle = crate::word_index::spawn();
    app.attach_word_index(handle.clone());
    let (lsp_handle, _queue) = crate::lsp::command_channel();
    app.lsp_workspace_allowed = true;
    app.attach_lsp(lsp_handle);
    app.prepare_view(word_completion_geometry());
    handle.flush();

    set_cursor(&mut app, 1, 0);
    press(&mut app, 'i');
    type_text(&mut app, "wob");
    assert_eq!(
        app.completion.as_ref().unwrap().source,
        CompletionSource::Word
    );

    // Ctrl-x's answer replaces an open Word popup, unlike Path's.
    let anchor = app.active().head();
    app.lsp_requests.insert(
        9,
        tracked(
            &app,
            PendingRequest::Completion {
                buffer: 0,
                anchor,
                explicit_session: None,
            },
        ),
    );
    app.apply_lsp_event(LspEvent::Response {
        token: 9,
        response: Response::Completions(vec![crate::lsp::Completion {
            label: "language_item".to_owned(),
            filter_text: None,
            sort_text: None,
            detail: String::new(),
            kind: "value",
            insert: "language_item".to_owned(),
            edit: None,
            additional: Vec::new(),
        }]),
    });
    assert_eq!(
        app.completion.as_ref().unwrap().source,
        CompletionSource::Language
    );

    // Word never opens over an active Language completion. "l" matches
    // "language_item" so the Language popup itself stays open too.
    type_text(&mut app, "l");
    assert_eq!(
        app.completion.as_ref().unwrap().source,
        CompletionSource::Language
    );

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn word_completion_yields_to_a_typed_path() {
    let root = temporary("word-completion-yields-to-path");
    fs::create_dir_all(root.join("dir")).unwrap();
    fs::write(root.join("dir").join("candidate.txt"), "").unwrap();
    let active = root.join("note.txt");
    fs::write(&active, "directory\n").unwrap();
    let mut app = App::new_in_project(Config::default(), Some(active), &root).unwrap();
    let handle = crate::word_index::spawn();
    app.attach_word_index(handle.clone());
    app.prepare_view(word_completion_geometry());
    handle.flush();

    set_cursor(&mut app, 1, 0);
    press(&mut app, 'i');
    type_text(&mut app, "dir");
    assert_eq!(
        app.completion.as_ref().unwrap().source,
        CompletionSource::Word
    );

    press(&mut app, '/');
    let state = app
        .completion
        .as_ref()
        .expect("path completion should have opened");
    assert_eq!(state.source, CompletionSource::Path);
    assert_eq!(state.selected_item().unwrap().label, "candidate.txt");

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn word_index_follows_buffer_open_edit_and_close() {
    let root = temporary("word-index-sync");
    fs::create_dir_all(&root).unwrap();
    let active = root.join("a.txt");
    fs::write(&active, "\n").unwrap();
    let other = root.join("b.txt");
    fs::write(&other, "gadget\n").unwrap();
    let mut app = App::new_in_project(Config::default(), Some(active), &root).unwrap();
    let handle = crate::word_index::spawn();
    app.attach_word_index(handle.clone());
    let other_id = app.host_open_file(other, false).unwrap();

    // Opening b.txt in the background is enough to index it: nothing
    // edits or activates it before the sweep.
    app.prepare_view(word_completion_geometry());
    handle.flush();

    set_cursor(&mut app, 0, 0);
    press(&mut app, 'i');
    type_text(&mut app, "gad");
    let state = app.completion.as_ref().expect("word popup should be open");
    assert_eq!(state.source, CompletionSource::Word);
    assert_eq!(state.items[0].label, "gadget");

    key(&mut app, KeyCode::Escape, Modifiers::NONE);
    key(&mut app, KeyCode::Escape, Modifiers::NONE);
    app.buffers[0].set_text("\n");
    app.panes.get_mut(&0).unwrap().selection = Selection::point(0);

    app.close_buffer(other_id);
    handle.flush();

    press(&mut app, 'i');
    type_text(&mut app, "gad");
    assert!(app.completion.is_none());

    fs::remove_dir_all(root).unwrap();
}

/// Undo, redo, file reload, and a Git-triggered reload all replace a
/// buffer's whole text through `resync_replaced_buffer` rather than
/// through `apply_to_buffer`'s transactional path, so that is the only
/// place those operations can reindex it. This exercises undo; redo and
/// the reload paths share the exact same call, not a parallel one.
#[test]
fn word_index_resyncs_after_undo() {
    let root = temporary("word-index-resync-undo");
    fs::create_dir_all(&root).unwrap();
    let active = root.join("a.txt");
    fs::write(&active, "gadget\n").unwrap();
    let mut app = App::new_in_project(Config::default(), Some(active), &root).unwrap();
    let handle = crate::word_index::spawn();
    app.attach_word_index(handle.clone());
    app.prepare_view(word_completion_geometry());
    handle.flush();

    set_cursor(&mut app, 0, 6);
    press(&mut app, 'i');
    type_text(&mut app, " widget");
    key(&mut app, KeyCode::Escape, Modifiers::NONE);
    handle.flush();
    assert!(
        handle
            .current()
            .buffer_words(0)
            .unwrap()
            .entries()
            .iter()
            .any(|(word, _)| word == "widget"),
        "widget should be indexed after typing it"
    );

    // Undo removes "widget" from the buffer text; the index must not go
    // on offering it as a candidate until some later transactional edit
    // happens to reindex the buffer.
    press(&mut app, 'u');
    handle.flush();
    assert!(
        !handle
            .current()
            .buffer_words(0)
            .unwrap()
            .entries()
            .iter()
            .any(|(word, _)| word == "widget"),
        "undo should have dropped widget from the index"
    );

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn word_completion_queries_skip_a_typed_opening_wrapper() {
    let root = temporary("word-completion-wrapper-query");
    fs::create_dir_all(&root).unwrap();
    let active = root.join("a.txt");
    fs::write(&active, "background\n").unwrap();
    let mut app = App::new_in_project(Config::default(), Some(active), &root).unwrap();
    let handle = crate::word_index::spawn();
    app.attach_word_index(handle.clone());
    app.prepare_view(word_completion_geometry());
    handle.flush();

    set_cursor(&mut app, 1, 0);
    press(&mut app, 'i');
    // The index stores "background" with its backtick trimmed; typing
    // one here must not make it part of the query, or it could never
    // match.
    type_text(&mut app, "`bac");

    let state = app.completion.as_ref().expect("word popup should be open");
    assert_eq!(state.source, CompletionSource::Word);
    assert_eq!(state.selected_item().unwrap().label, "background");

    key(&mut app, KeyCode::Tab, Modifiers::NONE);
    assert!(app.completion.is_none());
    // The opening backtick is untouched; only "bac" (the part after it)
    // is what gets replaced.
    assert_eq!(text(&app), "background\n`background");

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn delayed_hover_does_not_take_enter_from_completion_or_a_prompt() {
    let mut completion = App::new(Config::default(), None).unwrap();
    press(&mut completion, 'i');
    completion.completion = Some(CompletionState {
        items: vec![crate::lsp::Completion {
            label: "accepted".to_owned(),
            filter_text: None,
            sort_text: None,
            detail: String::new(),
            kind: "text",
            insert: "accepted".to_owned(),
            edit: None,
            additional: Vec::new(),
        }],
        selected: 0,
        buffer: 0,
        anchor: 0,
        filter: String::new(),
        source: CompletionSource::Language,
        explicit_session: None,
    });
    completion.hover = Some(HoverState {
        lines: (0..20).map(|row| format!("hover {row}")).collect(),
    });

    key(&mut completion, KeyCode::Enter, Modifiers::NONE);

    // Completion no longer accepts on Enter (that's Tab's job), but the
    // point of this test survives: hover's own "peek expands to a full
    // page" handling for Enter must still stay deferred to the active
    // completion popup, which dismisses and lets Enter reach its usual
    // newline, rather than hover hijacking the key into opening a
    // documentation view.
    assert_eq!(text(&completion), "\n");
    assert!(completion.completion.is_none());
    assert!(completion.buffers.iter().all(|buffer| {
        buffer.generated_view_identity() != Some(&GeneratedViewIdentity::Documentation)
    }));

    let mut prompt = App::new(Config::default(), None).unwrap();
    seed(&mut prompt, "needle");
    press(&mut prompt, '/');
    type_text(&mut prompt, "needle");
    prompt.hover = Some(HoverState {
        lines: (0..20).map(|row| format!("hover {row}")).collect(),
    });

    key(&mut prompt, KeyCode::Enter, Modifiers::NONE);

    assert_eq!(prompt.mode, Mode::Select);
    assert_eq!(prompt.active().selection.primary().from(), 0);
    assert!(prompt.buffers.iter().all(|buffer| {
        buffer.generated_view_identity() != Some(&GeneratedViewIdentity::Documentation)
    }));
}

#[test]
fn hover_full_view_uses_the_prepared_editor_height() {
    let mut app = App::new(Config::default(), None).unwrap();
    app.hover = Some(HoverState {
        lines: (0..7).map(|row| format!("hover {row}")).collect(),
    });
    app.prepare_view(FrameGeometry {
        screen: Rect {
            x: 0,
            y: 0,
            width: 100,
            height: 10,
        },
        editor: Rect {
            x: 0,
            y: 0,
            width: 100,
            height: 8,
        },
        status: Rect {
            x: 0,
            y: 8,
            width: 100,
            height: 1,
        },
        message: Rect {
            x: 0,
            y: 9,
            width: 100,
            height: 1,
        },
    });
    let snapshot = app
        .overlay_snapshots()
        .into_iter()
        .find(|overlay| overlay.kind == crate::snapshot::OverlayKind::Hover)
        .unwrap();
    assert_eq!(snapshot.rows.len(), 6);
    assert_eq!(snapshot.omitted_rows, 1);
    assert!(
        snapshot
            .actions
            .iter()
            .any(|action| action.key_hint == "Enter")
    );

    key(&mut app, KeyCode::Enter, Modifiers::NONE);

    assert_eq!(
        app.active_buffer().generated_view_identity(),
        Some(&GeneratedViewIdentity::Documentation)
    );
}

#[test]
fn slash_completes_paths_from_the_file_directory_and_project_directory() {
    let base = temporary("path-completion");
    let project = base.join("project");
    let files = project.join("files");
    let local = files.join("dir");
    let sibling = base.join("some_dir");
    fs::create_dir_all(&local).unwrap();
    fs::create_dir_all(&sibling).unwrap();
    fs::write(files.join("a.txt"), "").unwrap();
    fs::write(local.join("b.txt"), "").unwrap();
    fs::write(sibling.join("outside.txt"), "").unwrap();

    let labels_after = |path: &str| {
        let mut app =
            App::new_in_project(Config::default(), Some(files.join("a.txt")), &project).unwrap();
        press(&mut app, 'i');
        type_text(&mut app, path);
        let state = app
            .completion
            .expect("a valid directory should open completion");
        assert_eq!(state.source, CompletionSource::Path);
        state
            .items
            .into_iter()
            .map(|item| item.label)
            .collect::<Vec<_>>()
    };

    assert!(labels_after(&format!("{}/", local.display())).contains(&"b.txt".to_owned()));
    assert!(labels_after("dir/").contains(&"b.txt".to_owned()));
    assert!(labels_after("files/").contains(&"a.txt".to_owned()));
    assert!(labels_after("./files/").contains(&"a.txt".to_owned()));
    assert!(labels_after("../files/").contains(&"a.txt".to_owned()));
    assert!(labels_after("../some_dir/").contains(&"outside.txt".to_owned()));

    fs::remove_dir_all(base).unwrap();
}

#[test]
fn path_completion_filters_filename_punctuation_and_continues_into_directories() {
    let root = temporary("nested-path-completion");
    let nested = root.join("files").join("dir");
    fs::create_dir_all(&nested).unwrap();
    fs::write(root.join("files").join("a-file.txt"), "").unwrap();
    fs::write(nested.join("deep.txt"), "").unwrap();
    let active = root.join("note.txt");
    fs::write(&active, "").unwrap();
    let mut app = App::new_in_project(Config::default(), Some(active), &root).unwrap();

    press(&mut app, 'i');
    type_text(&mut app, "files/a-");
    let state = app
        .completion
        .as_ref()
        .expect("punctuation keeps path completion open");
    assert_eq!(state.filter, "a-");
    assert_eq!(state.selected_item().unwrap().label, "a-file.txt");

    let mut app =
        App::new_in_project(Config::default(), Some(root.join("note.txt")), &root).unwrap();
    press(&mut app, 'i');
    type_text(&mut app, "files/di");
    assert_eq!(
        app.completion
            .as_ref()
            .unwrap()
            .selected_item()
            .unwrap()
            .label,
        "dir/"
    );
    key(&mut app, KeyCode::Tab, Modifiers::NONE);
    assert!(text(&app).ends_with("files/dir/"));
    assert_eq!(
        app.completion
            .as_ref()
            .unwrap()
            .selected_item()
            .unwrap()
            .label,
        "deep.txt"
    );

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn path_completion_reopens_while_editing_an_existing_path_without_retyping_slash() {
    let root = temporary("path-completion-mid-edit");
    fs::create_dir_all(root.join("dir")).unwrap();
    fs::write(root.join("dir").join("target.txt"), "").unwrap();
    let active = root.join("note.txt");
    fs::write(&active, "").unwrap();
    let mut app = App::new_in_project(Config::default(), Some(active), &root).unwrap();

    press(&mut app, 'i');
    type_text(&mut app, "dir/");
    assert_eq!(
        app.completion.as_ref().unwrap().source,
        CompletionSource::Path,
        "typing '/' still opens hints as before"
    );

    // Any editor command (caret movement, a fresh Insert session, ...)
    // clears the popup the same way; simulate that directly, leaving
    // the already-typed path text and caret untouched.
    app.completion = None;

    type_text(&mut app, "ta");
    let state = app
        .completion
        .as_ref()
        .expect("typing after an existing path should reopen hints without retyping '/'");
    assert_eq!(state.source, CompletionSource::Path);
    assert_eq!(state.filter, "ta");
    assert_eq!(state.selected_item().unwrap().label, "target.txt");

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn late_language_responses_do_not_replace_an_active_path_completion() {
    let root = temporary("path-completion-late-lsp");
    fs::create_dir_all(root.join("dir")).unwrap();
    fs::write(root.join("dir").join("candidate.txt"), "").unwrap();
    let active = root.join("note.rs");
    fs::write(&active, "").unwrap();
    let mut app = App::new_in_project(Config::default(), Some(active), &root).unwrap();
    let (handle, _queue) = crate::lsp::command_channel();
    app.lsp_workspace_allowed = true;
    app.attach_lsp(handle);
    ready(&mut app, Encoding::Utf8);
    press(&mut app, 'i');

    for (token, response) in [
        (
            41,
            Response::Completions(vec![crate::lsp::Completion {
                label: "language_item".to_owned(),
                filter_text: None,
                sort_text: None,
                detail: String::new(),
                kind: "value",
                insert: "language_item".to_owned(),
                edit: None,
                additional: Vec::new(),
            }]),
        ),
        (42, Response::Empty),
    ] {
        app.lsp_requests.insert(
            token,
            tracked(
                &app,
                PendingRequest::Completion {
                    buffer: 0,
                    anchor: 0,
                    explicit_session: None,
                },
            ),
        );
        if token == 41 {
            type_text(&mut app, "dir/");
        }
        app.apply_lsp_event(LspEvent::Response { token, response });
        let state = app
            .completion
            .as_ref()
            .expect("the path popup should survive a late language response");
        assert_eq!(state.source, CompletionSource::Path);
        assert_eq!(state.selected_item().unwrap().label, "candidate.txt");
    }

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn path_completion_bounds_directory_enumeration() {
    let root = temporary("bounded-path-completion");
    let huge = root.join("huge");
    fs::create_dir_all(&huge).unwrap();
    for index in 0..PATH_COMPLETION_ITEM_LIMIT_PER_ROOT + 16 {
        fs::write(huge.join(format!("candidate-{index:04}.txt")), "").unwrap();
    }
    let active = root.join("note.txt");
    fs::write(&active, "").unwrap();
    let mut app = App::new_in_project(Config::default(), Some(active), &root).unwrap();

    press(&mut app, 'i');
    type_text(&mut app, "huge/");

    assert_eq!(
        app.completion.as_ref().unwrap().items.len(),
        PATH_COMPLETION_ITEM_LIMIT_PER_ROOT
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_full_file_directory_does_not_hide_project_root_path_candidates() {
    let root = temporary("both-bounded-path-roots");
    let project = root.join("project");
    let file_directory = project.join("files");
    let local_candidates = file_directory.join("dir");
    let project_candidates = project.join("dir");
    fs::create_dir_all(&local_candidates).unwrap();
    fs::create_dir_all(&project_candidates).unwrap();
    for index in 0..PATH_COMPLETION_ITEM_LIMIT_PER_ROOT + 16 {
        fs::write(local_candidates.join(format!("local-{index:04}.txt")), "").unwrap();
    }
    fs::write(project_candidates.join("project-only.txt"), "").unwrap();
    let active = file_directory.join("note.txt");
    fs::write(&active, "").unwrap();
    let mut app = App::new_in_project(Config::default(), Some(active), &project).unwrap();

    press(&mut app, 'i');
    type_text(&mut app, "dir/");

    assert!(
        app.completion
            .as_ref()
            .unwrap()
            .items
            .iter()
            .any(|item| item.label == "project-only.txt")
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_stale_completion_response_is_discarded_rather_than_shown() {
    let (mut app, _, _queue) = rust_app("abc\n");
    ready(&mut app, Encoding::Utf8);
    // The person left insert mode while the request was in flight.
    app.lsp_requests.insert(
        4,
        tracked(
            &app,
            PendingRequest::Completion {
                buffer: 0,
                anchor: 0,
                explicit_session: None,
            },
        ),
    );
    app.apply_lsp_event(LspEvent::Response {
        token: 4,
        response: Response::Completions(vec![crate::lsp::Completion {
            label: "abc".to_owned(),
            filter_text: None,
            sort_text: None,
            detail: String::new(),
            kind: "",
            insert: "abc".to_owned(),
            edit: None,
            additional: Vec::new(),
        }]),
    });
    assert!(app.completion.is_none());
}

#[test]
fn language_completion_enters_insert_mode_and_requests_candidates() {
    let (mut app, _, mut queue) = rust_app("value\n");
    ready(&mut app, Encoding::Utf8);
    drain(&mut queue);
    set_cursor(&mut app, 0, 2);

    press(&mut app, ' ');
    press(&mut app, 'l');
    press(&mut app, 'c');

    assert_eq!(app.mode, Mode::Insert);
    let commands = drain(&mut queue);
    assert!(
        commands.iter().any(|command| matches!(
            command,
            LspCommand::Request { kind, .. }
                if matches!(kind.as_ref(), RequestKind::Completion(_))
        )),
        "{commands:?}"
    );
}

#[test]
fn a_new_transient_request_cancels_the_superseded_one() {
    let (mut app, _path, mut queue) = rust_app("value\n");
    ready(&mut app, Encoding::Utf8);
    drain(&mut queue);

    app.lsp_hover();
    let first = drain(&mut queue);
    let token = first
        .iter()
        .find_map(|command| match command {
            LspCommand::Request { token, .. } => Some(*token),
            _ => None,
        })
        .unwrap();
    app.lsp_hover();
    let second = drain(&mut queue);
    assert!(second.iter().any(
        |command| matches!(command, LspCommand::Cancel { token: cancelled } if *cancelled == token)
    ));
    assert_eq!(app.lsp_requests.len(), 1);
}

#[test]
fn malformed_or_overlapping_completion_edits_change_nothing() {
    for (primary, additional) in [
        (edit(0, 1, 1, "x"), vec![edit(7, 0, 0, "outside")]),
        (edit(0, 0, 2, "x"), vec![edit(0, 1, 3, "overlap")]),
    ] {
        let (mut app, _path, mut queue) = rust_app("abc\n");
        ready(&mut app, Encoding::Utf8);
        drain(&mut queue);
        set_cursor(&mut app, 0, 1);
        press(&mut app, 'i');
        let anchor = app.active().head();
        app.completion = Some(CompletionState {
            items: vec![crate::lsp::Completion {
                label: "candidate".to_owned(),
                filter_text: None,
                sort_text: None,
                detail: String::new(),
                kind: "value",
                insert: "candidate".to_owned(),
                edit: Some((primary.range, primary.new_text)),
                additional,
            }],
            selected: 0,
            buffer: 0,
            anchor,
            filter: String::new(),
            source: CompletionSource::Language,
            explicit_session: None,
        });
        key(&mut app, KeyCode::Tab, Modifiers::NONE);
        assert_eq!(text(&app), "abc\n");
        assert!(app.status_error);
    }
}

#[test]
fn completion_caret_follows_the_primary_edit_not_a_later_additional_edit() {
    let (mut app, _path, mut queue) = rust_app("abc\n");
    ready(&mut app, Encoding::Utf8);
    drain(&mut queue);
    press(&mut app, 'i');
    app.completion = Some(CompletionState {
        items: vec![crate::lsp::Completion {
            label: "primary".to_owned(),
            filter_text: None,
            sort_text: None,
            detail: String::new(),
            kind: "value",
            insert: "primary".to_owned(),
            edit: Some((
                LspRange::new(LspPosition::new(0, 0), LspPosition::new(0, 1)),
                "primary".to_owned(),
            )),
            additional: vec![edit(0, 3, 3, "!")],
        }],
        selected: 0,
        buffer: 0,
        anchor: 0,
        filter: String::new(),
        source: CompletionSource::Language,
        explicit_session: None,
    });

    key(&mut app, KeyCode::Tab, Modifiers::NONE);

    assert_eq!(text(&app), "primarybc!\n");
    assert_eq!(app.active().head(), "primary".chars().count());
}

#[test]
fn a_response_nothing_is_waiting_for_is_ignored() {
    let (mut app, _, _queue) = rust_app("abc\n");
    app.apply_lsp_event(LspEvent::Response {
        token: 4242,
        response: Response::Hover("stale".to_owned()),
    });
    assert!(app.hover.is_none());
    assert!(!app.status_error);
}

#[test]
fn language_server_commands_degrade_to_a_message_without_a_server() {
    let mut app = App::new(Config::default(), None).unwrap();
    let sequences: &[&[char]] = &[
        &['g', 'd'],
        &['g', 'D'],
        &['g', 'y'],
        &['g', 'i'],
        &['g', 'r'],
        &[' ', 'l', 'h'],
        &[' ', 'l', 'c'],
        &[' ', 'l', 's'],
        &[' ', 'l', 'S'],
        &[' ', 'l', 'a'],
        &[' ', 'l', 'r'],
    ];
    for sequence in sequences {
        for character in *sequence {
            press(&mut app, *character);
        }
        assert!(
            app.status_error,
            "{sequence:?} should report why it cannot run"
        );
        assert_eq!(app.mode, Mode::Normal);
    }
    // Diagnostics is answered from local state, so it is not an error.
    press(&mut app, ' ');
    press(&mut app, 'l');
    press(&mut app, 'd');
    assert_eq!(app.status, "no diagnostics");
}

#[test]
fn the_rename_prompt_seeds_the_word_under_the_caret() {
    let (mut app, _, mut queue) = rust_app("let value = 1;\n");
    ready(&mut app, Encoding::Utf8);
    drain(&mut queue);
    set_cursor(&mut app, 0, 5);

    press(&mut app, ' ');
    press(&mut app, 'l');
    press(&mut app, 'r');
    assert_eq!(app.mode, Mode::Command);
    assert_eq!(app.prompt_kind, PromptKind::Rename);
    assert_eq!(app.command, "value");

    type_text(&mut app, "2");
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    let commands = drain(&mut queue);
    assert!(
            commands.iter().any(|command| matches!(
                command,
                LspCommand::Request { kind, .. }
                    if matches!(kind.as_ref(), RequestKind::Rename { new_name, .. } if new_name == "value2")
            )),
            "{commands:?}"
        );
}

#[test]
fn a_document_symbol_picker_filters_and_jumps() {
    let (mut app, path, _queue) = rust_app("fn alpha() {}\nfn beta() {}\n");
    ready(&mut app, Encoding::Utf8);
    app.lsp_requests.insert(
        2,
        tracked(
            &app,
            PendingRequest::Symbols {
                title: "Document symbols",
                path: path.clone(),
            },
        ),
    );
    app.apply_lsp_event(LspEvent::Response {
        token: 2,
        response: Response::Symbols(vec![
            crate::lsp::SymbolEntry {
                name: "alpha".to_owned(),
                kind: "function",
                container: String::new(),
                // Hierarchical symbols carry no path of their own.
                location: crate::lsp::Location {
                    path: PathBuf::new(),
                    range: LspRange::new(LspPosition::new(0, 3), LspPosition::new(0, 8)),
                    encoding: Encoding::Utf8,
                },
            },
            crate::lsp::SymbolEntry {
                name: "beta".to_owned(),
                kind: "function",
                container: String::new(),
                location: crate::lsp::Location {
                    path: PathBuf::new(),
                    range: LspRange::new(LspPosition::new(1, 3), LspPosition::new(1, 7)),
                    encoding: Encoding::Utf8,
                },
            },
        ]),
    });

    press(&mut app, 'b');
    press(&mut app, 'e');
    assert_eq!(app.list.as_ref().unwrap().visible_indices().len(), 1);
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    let selection = app.active().selection.primary();
    assert_eq!(
        app.active_buffer().position_of(selection.from()),
        Position::new(1, 3),
        "the picker jumped to the wrong symbol"
    );
}

/// The `lsp` row of a service-health report, which is the row the state of a
/// language server actually reaches a person through.
fn lsp_health(app: &App) -> (ServiceState, String) {
    let entry = app
        .service_health_snapshot()
        .entries
        .into_iter()
        .find(|entry| entry.service == "lsp")
        .expect("every report carries an lsp row");
    (entry.state, entry.detail)
}

#[test]
fn service_health_distinguishes_every_language_server_state_a_buffer_can_be_in() {
    let mut disabled_config = Config::default();
    disabled_config.lsp.enable = false;
    let mut disabled = App::new(disabled_config, None).unwrap();
    let (handle, _queue) = crate::lsp::command_channel();
    disabled.lsp_workspace_allowed = true;
    disabled.attach_lsp(handle);
    assert_eq!(
        lsp_health(&disabled),
        (ServiceState::Disabled, "disabled in settings".to_owned(),),
        "a manager attached under a disabled policy is still disabled"
    );

    let (mut app, _path, _queue) = rust_app("fn main() {}\n");
    assert_eq!(
        lsp_health(&app),
        (
            ServiceState::Idle,
            "rust server is configured and starting or stopped".to_owned(),
        ),
        "a configured server nobody has handshaken with is not ready"
    );

    ready(&mut app, Encoding::Utf8);
    assert_eq!(
        lsp_health(&app),
        (
            ServiceState::Ready,
            "rust server and document are attached".to_owned(),
        )
    );

    let (mut python, _python_queue) = {
        let mut app = App::new(Config::default(), None).unwrap();
        app.buffers[0].path = Some(temporary("script.py"));
        app.buffers[0].kind = crate::buffer::BufferKind::File;
        let (handle, queue) = crate::lsp::command_channel();
        app.lsp_workspace_allowed = true;
        app.attach_lsp(handle);
        (app, queue)
    };
    assert_eq!(
        lsp_health(&python),
        (
            ServiceState::Idle,
            "no server configured for active python buffer".to_owned(),
        ),
        "a recognized language without a configured server names the language"
    );

    python.buffers[0].path = Some(temporary("notes.unknown-suffix"));
    assert_eq!(
        lsp_health(&python),
        (
            ServiceState::Idle,
            "active buffer has no recognized language".to_owned(),
        )
    );
}

#[test]
fn service_health_reports_syntax_ready_only_once_the_active_buffer_has_a_tree() {
    let syntax_health = |app: &App| {
        let entry = app
            .service_health_snapshot()
            .entries
            .into_iter()
            .find(|entry| entry.service == "syntax")
            .expect("every report carries a syntax row");
        (entry.state, entry.detail)
    };

    let (mut app, _path, _queue) = rust_app("fn main() {}\n");
    assert_eq!(
        syntax_health(&app),
        (
            ServiceState::Idle,
            "active buffer is using plain text".to_owned(),
        ),
        "a buffer that has not been parsed yet is not reported as parsed"
    );

    app.reparse_whole(0);
    assert_eq!(
        syntax_health(&app),
        (
            ServiceState::Ready,
            "active buffer parsed successfully".to_owned(),
        )
    );
}

#[test]
fn workspace_lsp_permission_gates_all_servers_and_remembers_both_answers() {
    let root = temporary("workspace-permission");
    let project = root.join("project");
    let storage = root.join("private/trust");
    std::fs::create_dir_all(&project).unwrap();
    let mut app = App::new(Config::default(), None).unwrap();
    app.project_root = project.clone();
    app.buffers[0].path = Some(project.join("main.rs"));
    app.buffers[0].kind = BufferKind::File;
    app.configure_lsp_trust(Some(storage.clone()));
    assert!(!app.lsp_workspace_allowed);
    let overlays = app.overlay_snapshots();
    let choice = overlays
        .iter()
        .find(|overlay| overlay.kind == crate::snapshot::OverlayKind::ResultList)
        .unwrap();
    assert_eq!(
        choice.message.as_deref(),
        Some("LSP may execute project code")
    );
    assert!(
        choice
            .column_header
            .as_ref()
            .unwrap()
            .label
            .contains(project.to_str().unwrap())
    );
    assert!(
        app.list
            .as_ref()
            .unwrap()
            .title
            .contains("Run language servers")
    );
    assert!(
        app.list
            .as_ref()
            .unwrap()
            .selected_preview()
            .unwrap()
            .contains("may execute code")
    );
    let (handle, mut commands) = crate::lsp::command_channel();
    app.attach_lsp(handle);
    assert!(commands.try_recv().is_err());
    assert!(!app.lsp_send(LspCommand::Ensure {
        language: "custom".to_owned()
    }));
    assert!(commands.try_recv().is_err());
    // Esc dismisses the question without granting permission or persisting.
    app.handle_list_key(KeyStroke::plain(KeyCode::Escape))
        .unwrap();
    assert!(!app.lsp_workspace_allowed);
    assert_eq!(app.lsp_trust.as_ref().unwrap().load().unwrap(), None);
    app.open_lsp_trust();
    // Enter chooses the conservative initial row and remembers the refusal.
    app.handle_list_key(KeyStroke::plain(KeyCode::Enter))
        .unwrap();
    assert!(!app.lsp_workspace_allowed);
    assert!(app.list.is_none());
    assert_eq!(app.lsp_trust.as_ref().unwrap().load().unwrap(), Some(false));
    app.configure_lsp_trust(Some(storage.clone()));
    assert!(app.list.is_none());
    // The palette command remains available while the LSP manager is denied.
    for character in ":lsp-trust".chars() {
        app.handle_key(KeyStroke::plain(KeyCode::Char(character)))
            .unwrap();
    }
    app.handle_key(KeyStroke::plain(KeyCode::Enter)).unwrap();
    assert!(
        app.list
            .as_ref()
            .is_some_and(|list| list.title.starts_with("Run language servers")),
        "status: {}; mode: {:?}; command: {}",
        app.status,
        app.mode,
        app.command
    );
    // Temporary approval removes any durable decision.
    app.list.as_mut().unwrap().selected = 1;
    app.handle_list_key(KeyStroke::plain(KeyCode::Enter))
        .unwrap();
    assert!(app.lsp_workspace_allowed);
    assert!(matches!(commands.try_recv(), Ok(LspCommand::Ensure { .. })));
    assert_eq!(app.lsp_trust.as_ref().unwrap().load().unwrap(), None);
    app.open_lsp_trust();
    app.list.as_mut().unwrap().selected = 2;
    app.handle_list_key(KeyStroke::plain(KeyCode::Enter))
        .unwrap();
    assert_eq!(app.lsp_trust.as_ref().unwrap().load().unwrap(), Some(true));
    app.configure_lsp_trust(Some(storage.clone()));
    assert!(app.lsp_workspace_allowed);
    assert!(app.list.is_none());
    app.choose_lsp_trust(true, false);
    assert_eq!(app.lsp_trust.as_ref().unwrap().load().unwrap(), None);
    app.open_lsp_trust();
    app.handle_list_key(KeyStroke::plain(KeyCode::Enter))
        .unwrap();
    assert!(!app.lsp_workspace_allowed);
    assert!(app.lsp_documents.is_empty());
    assert!(app.lsp_servers.is_empty());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn workspace_lsp_permission_fails_closed_without_storage_and_respects_configuration() {
    let root = temporary("workspace-permission-unavailable");
    std::fs::create_dir_all(&root).unwrap();
    let mut app = App::new(Config::default(), None).unwrap();
    app.project_root = root.clone();
    app.configure_lsp_trust(None);
    app.choose_lsp_trust(true, true);
    assert!(!app.lsp_workspace_allowed);
    assert!(app.list.is_some());
    app.choose_lsp_trust(true, false);
    assert!(app.lsp_workspace_allowed);
    app.choose_lsp_trust(false, true);
    assert!(
        !app.lsp_workspace_allowed,
        "failed persistence still revokes this host"
    );
    app.config.lsp.enable = false;
    app.list = None;
    app.configure_lsp_trust(None);
    assert!(app.list.is_none());
    assert!(!app.lsp_workspace_allowed);
    std::fs::remove_dir_all(root).unwrap();
}
