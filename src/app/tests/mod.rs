// SPDX-License-Identifier: MPL-2.0

use std::{
    collections::HashSet,
    fs,
    rc::Rc,
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};

use super::*;
use crate::command::{
    CommandExposure, GRAMMAR_ONLY_EDITOR_COMMANDS, GrammarKind, INTERNAL_EDITOR_COMMANDS,
};
use crate::file_picker::ScanEntry;
use crate::keymap::{Binding, BindingAvailability, BindingTarget};
use crate::lsp::LspPosition;

fn key(app: &mut App, code: KeyCode, modifiers: Modifiers) {
    app.handle_key(KeyStroke::new(code, modifiers)).unwrap();
}

fn press(app: &mut App, character: char) {
    key(app, KeyCode::Char(character), Modifiers::NONE);
}

fn finish_macro_replay(app: &mut App) {
    let mut slices = 0;
    while app.macro_replay_pending() {
        app.advance_macro_replay().unwrap();
        slices += 1;
        assert!(
            slices <= 100,
            "macro replay did not reach a bounded outcome"
        );
    }
}

/// Opens `count` distinct clean generated pages so a retention test can reach
/// [`SPECIAL_BUFFER_RETENTION_LIMIT`] without hard-coding what that limit is.
///
/// Each page carries its own generated identity, so none of them reuses
/// another's buffer the way reopening one page would. Explorers cannot serve
/// here: a pane retargets the explorer it already has rather than adopting a
/// second one.
fn open_filler_special_buffers(app: &mut App, count: usize) -> Vec<usize> {
    (0..count)
        .map(|index| {
            app.open_virtual_page(
                GeneratedViewIdentity::Named(format!("retention-filler-{index}")),
                format!("[filler {index}]"),
                "generated page\n",
                ContentAlignment::default(),
            )
        })
        .collect()
}

fn context_action(app: &mut App, mnemonic: char) {
    key(app, KeyCode::Tab, Modifiers::NONE);
    press(app, mnemonic);
}

/// Seeds the active buffer through a transaction, the same path real
/// editing uses, and resets the selection to a caret at the start.
fn seed(app: &mut App, text: &str) {
    app.buffers[0].apply(&Transaction::insert(0, text));
    app.panes.get_mut(&0).unwrap().selection = Selection::point(0);
}

fn text(app: &App) -> String {
    app.active_buffer().to_string()
}

fn syntax_language_name(app: &App, buffer_id: usize) -> Option<&'static str> {
    app.syntax[buffer_id]
        .as_ref()
        .map(DocumentSyntax::language)
        .map(|language| app.registry.language_name(language))
}

fn cursor(app: &App) -> Position {
    app.cursor_position()
}

fn set_cursor(app: &mut App, row: usize, col: usize) {
    let offset = app.active_buffer().offset_of(Position::new(row, col));
    app.panes.get_mut(&0).unwrap().selection = Selection::point(offset);
}

fn confirmation_snapshot(app: &App) -> crate::snapshot::OverlaySnapshot {
    let overlay = app
        .overlay_snapshots()
        .into_iter()
        .find(|overlay| overlay.kind == crate::snapshot::OverlayKind::Confirmation)
        .expect("a generic confirmation overlay");
    let message = overlay.message.as_deref().expect("a confirmation message");
    assert!(
        message.contains('\n'),
        "sentences use separate lines: {message:?}"
    );
    assert!(
        message.chars().next().is_some_and(char::is_uppercase),
        "confirmation messages start with a capital letter: {message:?}"
    );
    assert!(
        message
            .split('\n')
            .all(|line| matches!(line.chars().last(), Some('.' | '?' | '!'))),
        "each confirmation line is a complete sentence: {message:?}"
    );
    overlay
}

fn launch_position(line: usize, column: Option<usize>) -> LaunchPosition {
    LaunchPosition {
        line: std::num::NonZeroUsize::new(line).unwrap(),
        column: column.map(|column| std::num::NonZeroUsize::new(column).unwrap()),
    }
}

/// The system temporary directory under its filesystem identity.
///
/// macOS commonly advertises `/var/...` through `TMPDIR` while resolving the
/// same directory as `/private/var/...`. Application paths are canonicalized
/// when they become buffer and workspace identities, so fixtures must begin
/// from that same spelling or assertions accidentally compare aliases.
fn temporary_directory() -> PathBuf {
    std::env::temp_dir()
        .canonicalize()
        .unwrap_or_else(|_| std::env::temp_dir())
}

/// All production source that implements the application coordinator.
fn production_source() -> String {
    fn collect_modules(directory: &std::path::Path, sources: &mut Vec<std::path::PathBuf>) {
        for entry in fs::read_dir(directory).expect("application source directory") {
            let path = entry.expect("application source entry").path();
            if path.is_dir() {
                if path.file_name().is_some_and(|name| name != "tests") {
                    collect_modules(&path, sources);
                }
            } else if path.extension().is_some_and(|extension| extension == "rs") {
                sources.push(path);
            }
        }
    }

    let source_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let app = fs::read_to_string(source_root.join("app.rs"))
        .expect("application coordinator source")
        .split("\n#[cfg(test)]\nmod tests")
        .next()
        .unwrap()
        .to_owned();
    let mut paths = Vec::new();
    collect_modules(&source_root.join("app"), &mut paths);
    paths.sort();

    std::iter::once(app)
        .chain(paths.into_iter().map(|path| {
            fs::read_to_string(&path)
                .unwrap_or_else(|error| panic!("could not read {}: {error}", path.display()))
        }))
        .collect::<Vec<_>>()
        .join("\n")
}

mod commands;
mod comparisons;
mod editing;
mod editing_and_buffers;
mod git;
mod git_discovery;
mod language;
mod navigation_and_files;
mod presentation_and_settings;
mod search_and_pickers;
mod tutorial;
mod workspace;

use commands::{type_command, type_text, vim_app};
use editing_and_buffers::MemoryClipboard;
use language::{drain, ready, rust_app, temporary, tracked};

#[test]
fn configured_leader_is_owned_by_the_app_and_space_becomes_insertable() {
    let config = Config {
        keys: Some(serde_yaml::from_str("leader: Ctrl-x\n").expect("valid raw key configuration")),
        ..Config::default()
    };
    let mut app = App::new(config, None).unwrap();
    assert_eq!(app.keymap().leader(), KeyStroke::ctrl('x'));
    assert!(matches!(
        app.keymap().lookup(
            Mode::Normal,
            &KeySequence::parse("Ctrl-x e").expect("valid sequence")
        ),
        crate::keymap::Lookup::Exact(binding)
            if binding.target == BindingTarget::Editor(EditorCommand::OpenExplorer)
    ));
    assert!(matches!(
        app.keymap().lookup(
            Mode::Normal,
            &KeySequence::parse("Space e").expect("valid sequence")
        ),
        crate::keymap::Lookup::NoMatch
    ));

    app.mode = Mode::Insert;
    press(&mut app, ' ');
    assert_eq!(text(&app), " ");
}

#[test]
fn malformed_key_configuration_reports_an_error_and_keeps_defaults() {
    let config = Config {
        keys: Some(serde_yaml::Value::Bool(true)),
        ..Config::default()
    };
    let app = App::new(config, None).unwrap();
    assert_eq!(app.keymap().leader(), KeyStroke::char(' '));
    let entries = app.notifications.entries();
    assert_eq!(
        entries
            .iter()
            .filter(|entry| entry.title == "Key bindings")
            .count(),
        1
    );
    assert!(entries[0].body.contains("expected a mapping"));
    assert!(
        !entries
            .iter()
            .any(|entry| entry.title == "Startup configuration")
    );
    assert!(app.status.contains("1 key binding entries rejected"));
}

#[test]
fn explicit_null_key_configuration_is_reported_instead_of_treated_as_absent() {
    let config: Config = serde_yaml::from_str("keys: null\n").unwrap();
    let app = App::new(config, None).unwrap();
    let notification = app
        .notifications
        .entries()
        .iter()
        .find(|entry| entry.title == "Key bindings")
        .expect("an explicit null keys section is rejected");
    assert!(notification.body.contains("expected a mapping"));
}

#[test]
fn variant_specific_key_rejection_is_named_once_before_settings_change() {
    let config = Config {
        keys: Some(
            serde_yaml::from_str("rebind:\n  Space e: Ctrl-h\n")
                .expect("valid raw key configuration"),
        ),
        ..Config::default()
    };
    let app = App::new(config, None).unwrap();
    let notification = app
        .notifications
        .entries()
        .iter()
        .find(|entry| entry.title == "Key bindings")
        .expect("variant rejection is reported");
    assert!(notification.body.contains("fast_pane_keys=true:"));
    assert!(!notification.body.contains("fast_pane_keys=false:"));
    assert!(app.status.contains("1 key binding entries rejected"));
}

#[test]
fn configured_spellings_reach_about_manual_help_and_the_tutorial() {
    let configured = || Config {
        keys: Some(
            serde_yaml::from_str("leader: Ctrl-x\nwindow: Ctrl-a\n")
                .expect("valid raw key configuration"),
        ),
        ..Config::default()
    };

    let mut about = App::new(configured(), None).unwrap();
    about.execute_command("about").unwrap();
    assert!(text(&about).contains("Ctrl-x ?"));
    assert!(text(&about).contains("Ctrl-x f"));

    let mut manual = App::new(configured(), None).unwrap();
    manual.execute_command("help").unwrap();
    assert!(text(&manual).contains("Use Ctrl-x ? for contextual help"));
    assert!(text(&manual).contains("Press Ctrl-x and pause"));

    let mut contextual = App::new(configured(), None).unwrap();
    key(&mut contextual, KeyCode::Char('x'), Modifiers::CONTROL);
    press(&mut contextual, '?');
    assert!(text(&contextual).contains("Ctrl-x ?"));
    assert!(text(&contextual).contains("Ctrl-a"));

    let mut tutorial = App::new(configured(), None).unwrap();
    tutorial.execute_command("tutorial").unwrap();
    key(&mut tutorial, KeyCode::Enter, Modifiers::NONE);
    tutorial.tutorial.as_mut().unwrap().lesson = 8;
    tutorial.refresh_tutorial_document();
    let instructions = tutorial.tutorial.as_ref().unwrap().instruction_buffer;
    let instructions = tutorial.buffers[instructions].to_string();
    assert!(instructions.contains("Ctrl-x s c"));
    assert!(instructions.contains("Ctrl-a"));
}
