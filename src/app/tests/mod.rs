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
mod language;
mod navigation_and_files;
mod presentation_and_settings;
mod search_and_pickers;
mod tutorial;
mod workspace;

use commands::{type_command, type_text, vim_app};
use editing_and_buffers::MemoryClipboard;
use language::{drain, ready, rust_app, temporary, tracked};
