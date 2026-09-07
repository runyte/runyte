// SPDX-License-Identifier: MPL-2.0

use super::*;
use crate::text::Change;

struct SyntaxFixture(PathBuf);
impl SyntaxFixture {
    fn new(label: &str) -> Self {
        let root = temporary(label);
        fs::create_dir_all(&root).unwrap();
        Self(root)
    }
    fn file(&self, name: &str, text: &str) -> PathBuf {
        let path = self.0.join(name);
        fs::write(&path, text).unwrap();
        path
    }
    fn app(&self, targets: Vec<PathBuf>) -> App {
        App::new_with_boundaries(
            Config::default(),
            targets.into_iter().map(LaunchTarget::new).collect(),
            self.0.clone(),
            &mut StartupTrace::new(),
            self.0.clone(),
            ProgramCache::default(),
            HostPorts::isolated(Box::new(MemoryClipboard(Arc::new(Mutex::new(
                String::new(),
            ))))),
            true,
        )
        .unwrap()
    }
}
impl Drop for SyntaxFixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn finished(app: &App, buffer: usize) -> SyntaxEvent {
    let request = &app.pending_syntax[&buffer];
    SyntaxEvent {
        buffer,
        generation: request.generation,
        language: request.language,
        text_revision: request.target.revision(),
        syntax: DocumentSyntax::new(&request.target, request.language, &app.registry),
        failure: None,
    }
}

#[test]
fn deferred_startup_is_editable_before_any_parser_runs() {
    let fixture = SyntaxFixture::new("async-initial-editing");
    let path = fixture.file("main.rs", "fn main() {\n    let café = 1;\n}\n");
    let mut app = fixture.app(vec![path.clone()]);
    assert!(app.has_pending_syntax());
    assert!(app.syntax.iter().all(Option::is_none));
    assert_eq!(text(&app), fs::read_to_string(&path).unwrap());
    assert!(app.highlights(0, 0, app.buffers[0].len_chars()).is_empty());
    let before = text(&app);
    // Two simultaneous edits, including non-ASCII text, before a tree exists.
    app.apply_to_buffer(
        0,
        &Transaction::new(vec![
            Change::new(0, 0, "// α\n"),
            Change::new(before.chars().count(), before.chars().count(), "// ω\n"),
        ]),
    );
    let expected = format!("// α\n{before}// ω\n");
    assert_eq!(text(&app), expected);
    assert_eq!(
        app.pending_syntax[&0].target.revision(),
        app.buffers[0].text().revision()
    );
    app.save(None, false).unwrap();
    assert_eq!(fs::read_to_string(path).unwrap(), expected);
    let event = finished(&app, 0);
    app.undo();
    assert!(!app.apply_syntax_event(event));
    assert_eq!(text(&app), before);
    app.redo();
    assert_eq!(text(&app), expected);
    let selection = app.active().selection.clone();
    let scroll = (app.active().scroll_row, app.active().scroll_col);
    assert!(app.apply_syntax_event(finished(&app, 0)));
    assert_eq!(text(&app), expected);
    assert_eq!(app.active().selection, selection);
    assert_eq!((app.active().scroll_row, app.active().scroll_col), scroll);
    assert!(!app.highlights(0, 0, app.buffers[0].len_chars()).is_empty());
}

#[test]
fn pending_structural_commands_refuse_without_moving_or_replaying() {
    let fixture = SyntaxFixture::new("async-commands");
    let path = fixture.file("main.rs", "fn main() {}\n");
    let mut app = fixture.app(vec![path]);
    let selection = app.active().selection.clone();
    for command in EditorCommand::ALL
        .iter()
        .copied()
        .filter(|command| command.capability() == Some(crate::command::CommandCapability::Syntax))
    {
        let invocation = crate::command::CommandInvocation::editor(
            command,
            crate::command::CommandExecutionContext::default(),
        )
        .unwrap();
        assert!(
            matches!(app.execute(invocation).unwrap(), CommandOutcome::Unavailable(reason) if reason == "Syntax is still parsing")
        );
        assert_eq!(app.active().selection, selection);
    }
    assert_eq!(
        app.command_capabilities().syntax.reason(),
        Some("Syntax is still parsing")
    );
    assert!(
        app.service_health_snapshot()
            .entries
            .iter()
            .any(|entry| entry.service == "syntax" && entry.detail == "Syntax is still parsing")
    );
    assert!(app.apply_syntax_event(finished(&app, 0)));
    assert_eq!(app.active().selection, selection);
    assert!(app.command_capabilities().syntax.is_available());
}

#[test]
fn stale_initial_results_and_failures_cannot_replace_newer_text_or_language() {
    let fixture = SyntaxFixture::new("async-generation");
    let path = fixture.file("script", "#!/usr/bin/env lua\nprint('hello')\n");
    let mut app = fixture.app(vec![path]);
    let old = finished(&app, 0);
    app.apply_to_buffer(0, &Transaction::insert(0, "# moved shebang\n"));
    assert!(!app.has_pending_syntax());
    assert!(!app.apply_syntax_event(old.clone()));
    app.undo();
    assert!(app.has_pending_syntax());
    assert!(!app.apply_syntax_event(old));
    let mut failed = finished(&app, 0);
    failed.syntax = None;
    failed.failure = Some("parse timeout".into());
    assert!(app.apply_syntax_event(failed));
    assert!(!app.has_pending_syntax());
    assert_eq!(
        app.command_capabilities().syntax.reason(),
        Some("parse timeout")
    );
    app.apply_to_buffer(
        0,
        &Transaction::insert(app.buffers[0].len_chars(), "# still editable\n"),
    );
    assert!(text(&app).ends_with("# still editable\n"));
    app.save(None, false).unwrap();
    assert!(!app.has_pending_syntax());
}

#[test]
fn save_preserves_pending_or_ready_syntax_and_save_as_changes_language() {
    let fixture = SyntaxFixture::new("async-save");
    let path = fixture.file("main.rs", "fn main() {}\n");
    let mut app = fixture.app(vec![path]);
    let generation = app.pending_syntax[&0].generation;
    app.save(None, false).unwrap();
    assert_eq!(app.pending_syntax[&0].generation, generation);
    let old = finished(&app, 0);
    app.save(Some(fixture.0.join("main.py")), false).unwrap();
    assert_ne!(app.pending_syntax[&0].generation, generation);
    assert!(!app.apply_syntax_event(old));
    assert!(app.apply_syntax_event(finished(&app, 0)));
    let revision = app.syntax[0].as_ref().unwrap().revision();
    app.save(None, false).unwrap();
    assert!(!app.has_pending_syntax());
    assert_eq!(app.syntax[0].as_ref().unwrap().revision(), revision);
}

#[test]
fn protocol_open_is_atomic_and_closed_buffers_reject_completion() {
    let fixture = SyntaxFixture::new("async-host-open");
    let a = fixture.file("a.rs", "fn a() {}\n");
    let b = fixture.file("b.py", "print('b')\n");
    let mut app = fixture.app(vec![]);
    let ids = app.host_open_files(vec![a.clone(), b], true).unwrap();
    assert_eq!(app.pending_syntax.len(), 2);
    assert!(ids.iter().all(|id| app.syntax[*id].is_none()));
    let count = app.buffers.len();
    let c = fixture.file("c.rs", "fn c() {}\n");
    let invalid = fixture.file("binary.bin", "\0binary");
    assert!(app.host_open_files(vec![c, invalid], false).is_err());
    assert_eq!(app.buffers.len(), count);
    assert_eq!(app.pending_syntax.len(), 2);
    let event = finished(&app, ids[0]);
    app.close_buffer(ids[0]);
    assert!(!app.pending_syntax.contains_key(&ids[0]));
    assert!(!app.apply_syntax_event(event));
}

#[tokio::test]
async fn attaching_a_worker_completes_initial_requests_and_stops_cleanly() {
    let fixture = SyntaxFixture::new("async-attach");
    let a = fixture.file("a.rs", "fn a() {}\n");
    let b = fixture.file("b.py", "print('b')\n");
    let mut app = fixture.app(vec![a, b]);
    let (worker, mut events) = crate::syntax::spawn_background(Arc::clone(&app.registry));
    app.attach_syntax_worker(worker);
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while app.has_pending_syntax() {
            assert!(app.apply_syntax_event(events.recv().await.unwrap()));
        }
    })
    .await
    .unwrap();
    drop(events);
    app.reparse_whole(0);
    assert!(!app.has_pending_syntax());
    assert_eq!(
        app.command_capabilities().syntax.reason(),
        Some("syntax worker is unavailable")
    );
    app.apply_to_buffer(0, &Transaction::insert(0, "// edit\n"));
    assert!(text(&app).starts_with("// edit\n"));
}
