// SPDX-License-Identifier: MPL-2.0

use std::{
    fs,
    num::NonZeroUsize,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use runyte::{
    app::CommandOutcome,
    command::{
        CommandExecutionContext, CommandId, CommandInvocation, EditorCommand, GrammarKind,
        InvocationParameters, parse_colon_command,
    },
    headless::HeadlessEditor,
    selection::{Range, Selection},
    text::{Change, Transaction},
};

fn editor_invocation(command: EditorCommand) -> CommandInvocation {
    CommandInvocation::editor(command, CommandExecutionContext::default()).unwrap()
}

fn selected_text(editor: &HeadlessEditor) -> String {
    let selection = editor.active_selection().primary();
    editor
        .active_text()
        .chars()
        .skip(selection.from())
        .take(selection.len())
        .collect()
}

fn inclusive_selected_text(editor: &HeadlessEditor) -> String {
    let selection = editor.active_selection().primary();
    editor
        .active_text()
        .chars()
        .skip(selection.from())
        .take(selection.len() + 1)
        .collect()
}

fn inclusive_selection_pieces(editor: &HeadlessEditor) -> Vec<String> {
    let text = editor.active_text();
    editor
        .active_selection()
        .ranges()
        .iter()
        .map(|range| {
            text.chars()
                .skip(range.from())
                .take(range.len() + 1)
                .collect()
        })
        .collect()
}

fn char_offset(source: &str, needle: &str) -> usize {
    source[..source.find(needle).unwrap()].chars().count()
}

fn head_character(editor: &HeadlessEditor) -> Option<char> {
    editor
        .active_text()
        .chars()
        .nth(editor.active_selection().primary().head)
}

fn assert_delete_uses_exact_selection(editor: &mut HeadlessEditor, inclusive: bool) {
    let before = editor.active_text();
    let range = editor.active_selection().primary();
    assert!(!range.is_empty(), "the structural command must select text");
    let expected = before
        .chars()
        .take(range.from())
        .chain(before.chars().skip(range.to() + usize::from(inclusive)))
        .collect::<String>();
    editor
        .execute(editor_invocation(EditorCommand::DeleteSelection))
        .unwrap();
    assert_eq!(editor.active_text(), expected);
}

static NEXT_ROOT: AtomicU64 = AtomicU64::new(1);

struct IsolatedRoot(PathBuf);

impl IsolatedRoot {
    fn new(label: &str) -> Self {
        let sequence = NEXT_ROOT.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "runyte-headless-{label}-{}-{sequence}",
            std::process::id()
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for IsolatedRoot {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

struct CurrentDirectoryGuard(PathBuf);

impl Drop for CurrentDirectoryGuard {
    fn drop(&mut self) {
        std::env::set_current_dir(&self.0).unwrap();
    }
}

fn relative_path(from: &Path, to: &Path) -> PathBuf {
    let from = from.components().collect::<Vec<_>>();
    let to = to.components().collect::<Vec<_>>();
    let common = from
        .iter()
        .zip(&to)
        .take_while(|(left, right)| left == right)
        .count();
    let mut relative = PathBuf::new();
    for _ in common..from.len() {
        relative.push("..");
    }
    for component in &to[common..] {
        relative.push(component.as_os_str());
    }
    relative
}

#[test]
fn public_headless_edit_command_selection_and_undo_flow() {
    let root = IsolatedRoot::new("flow");
    let mut editor = HeadlessEditor::with_text_in(root.path(), "abc").unwrap();
    assert_eq!(editor.active_text(), "abc");
    assert_eq!(editor.active_selection(), Selection::point(0));

    let move_right =
        CommandInvocation::editor(EditorCommand::MoveRight, CommandExecutionContext::default())
            .unwrap();
    editor.execute(move_right).unwrap();
    assert_eq!(editor.active_selection(), Selection::point(1));

    assert!(
        editor
            .apply_transaction(Transaction::insert(1, "X"))
            .unwrap()
    );
    assert_eq!(editor.active_text(), "aXbc");
    editor.undo().unwrap();
    assert_eq!(editor.active_text(), "abc");

    // `with_text` seeded through the same transaction history rather than by
    // replacing the backing text directly.
    editor.undo().unwrap();
    assert_eq!(editor.active_text(), "");
}

#[test]
fn headless_grammar_report_uses_typed_colon_identity_without_key_events() {
    let root = IsolatedRoot::new("grammar");
    let mut editor = HeadlessEditor::new_in(root.path()).unwrap();
    assert_eq!(editor.grammar_kind(), GrammarKind::Runyte);
    let invocation = CommandInvocation::from_parts(
        CommandId::Colon(runyte::command::ColonCommand::Grammar),
        InvocationParameters::Grammar(None),
        CommandExecutionContext::default(),
    )
    .unwrap();
    assert!(matches!(
        editor.execute(invocation).unwrap(),
        CommandOutcome::Status(message) if message == "active grammar: runyte"
    ));

    let removed = CommandInvocation::from_parts(
        CommandId::Colon(runyte::command::ColonCommand::Grammar),
        InvocationParameters::Grammar(Some(GrammarKind::Vim)),
        CommandExecutionContext::default(),
    )
    .unwrap();
    assert!(matches!(
        editor.execute(removed).unwrap(),
        CommandOutcome::UserError(message)
            if message == "vim grammar has been removed; use runyte"
    ));
    assert_eq!(editor.grammar_kind(), GrammarKind::Runyte);
}

#[test]
fn headless_selection_assignment_preserves_direction_and_clamps_offsets() {
    let root = IsolatedRoot::new("selection");
    let mut editor = HeadlessEditor::with_text_in(root.path(), "abc").unwrap();
    editor.set_active_selection(Selection::single(Range::new(2, 0)));
    assert_eq!(editor.active_selection().primary(), Range::new(2, 0));

    editor.set_active_selection(Selection::point(99));
    assert_eq!(editor.active_selection(), Selection::point(2));
}

#[test]
fn construction_uses_only_the_explicit_isolated_root_without_writing_it() {
    let root = IsolatedRoot::new("construction");
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 0);

    let editor = HeadlessEditor::new_in(root.path()).unwrap();

    assert_eq!(editor.active_text(), "");
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 0);
}

#[test]
fn relative_isolated_root_is_fixed_before_later_cwd_changes() {
    let original_path = std::env::current_dir().unwrap();
    let root = IsolatedRoot::new("relative-root");
    let changed = IsolatedRoot::new("changed-cwd");
    let original = CurrentDirectoryGuard(original_path);
    fs::write(root.path().join("note.txt"), "isolated").unwrap();
    fs::write(changed.path().join("note.txt"), "wrong cwd").unwrap();
    let canonical_root = root.path().canonicalize().unwrap();
    let relative_root = relative_path(&original.0, &canonical_root);
    assert!(relative_root.is_relative());

    let mut editor = HeadlessEditor::new_in(&relative_root).unwrap();
    std::env::set_current_dir(changed.path()).unwrap();
    editor
        .execute(CommandInvocation::open(PathBuf::from("note.txt")).unwrap())
        .unwrap();

    assert_eq!(editor.active_text(), "isolated");
}

#[test]
fn invalid_transactions_are_rejected_atomically_without_history() {
    let root = IsolatedRoot::new("invalid-transactions");
    let mut editor = HeadlessEditor::with_text_in(root.path(), "abc").unwrap();

    let insert = editor
        .apply_transaction(Transaction::insert(4, "x"))
        .unwrap_err();
    assert_eq!((insert.from(), insert.to(), insert.text_len()), (4, 4, 3));
    assert_eq!(editor.active_text(), "abc");

    let delete = editor
        .apply_transaction(Transaction::delete(1, 4))
        .unwrap_err();
    assert_eq!((delete.from(), delete.to(), delete.text_len()), (1, 4, 3));
    assert_eq!(editor.active_text(), "abc");

    let reversed = Transaction::new(vec![Change {
        from: 2,
        to: 1,
        text: String::new(),
    }]);
    let reversed = editor.apply_transaction(reversed).unwrap_err();
    assert_eq!((reversed.from(), reversed.to()), (2, 1));
    assert_eq!(editor.active_text(), "abc");

    let mixed = Transaction::new(vec![Change::new(0, 0, "X"), Change::new(4, 4, "Y")]);
    let mixed = editor.apply_transaction(mixed).unwrap_err();
    assert_eq!(mixed.change_index(), 1);
    assert_eq!(editor.active_text(), "abc");

    editor.undo().unwrap();
    assert_eq!(
        editor.active_text(),
        "",
        "rejected transactions must not add undo history"
    );
}

#[test]
fn facade_source_does_not_import_frontends_runtimes_or_optional_services() {
    let source = include_str!("../src/headless.rs");
    for forbidden in [
        "ratatui",
        "crossterm",
        "tokio",
        "crate::lsp",
        "crate::input",
        "KeyStroke",
    ] {
        assert!(
            !source.contains(forbidden),
            "headless facade contains forbidden dependency {forbidden}"
        );
    }
    assert!(!source.contains("pub app:"), "backing App became public");
    assert!(
        !source.contains("impl Deref"),
        "backing App gained a Deref escape"
    );
    assert!(source.contains("InertClipboard"));
    assert!(source.contains("HostPorts::isolated"));
    assert!(!source.contains("App::new("));
    for backend in ["tree_house", "tree_sitter", "lsp_types"] {
        assert!(
            !source.contains(backend),
            "headless facade exposes backend type {backend}"
        );
    }
}

#[test]
fn headless_active_outline_returns_only_owned_revision_safe_items() {
    let root = IsolatedRoot::new("outline");
    let source = "mod café {\n    fn βeta() {}\n}\n";
    fs::write(root.path().join("outline.rs"), source).unwrap();
    let mut editor = HeadlessEditor::new_in(root.path()).unwrap();
    editor
        .execute(CommandInvocation::open(PathBuf::from("outline.rs")).unwrap())
        .unwrap();

    let outline = editor.active_outline().unwrap().unwrap();
    assert_eq!(
        outline
            .items
            .iter()
            .map(|item| item.name.as_ref())
            .collect::<Vec<_>>(),
        ["café", "βeta"]
    );
    assert_eq!(outline.items[1].parent, Some(0));
    assert_eq!(outline.items[1].target.revision, outline.revision);
    assert_eq!(
        editor
            .execute(editor_invocation(EditorCommand::DocumentOutline))
            .unwrap(),
        CommandOutcome::Completed
    );
}

#[test]
fn structural_expand_and_shrink_round_trip_real_syntax() {
    let root = IsolatedRoot::new("structural-round-trip");
    let source = "fn demo() {\n    let value = call(1);\n}\n";
    fs::write(root.path().join("sample.rs"), source).unwrap();
    let mut editor = HeadlessEditor::new_in(root.path()).unwrap();
    editor
        .execute(CommandInvocation::open(PathBuf::from("sample.rs")).unwrap())
        .unwrap();
    let original = Selection::point(source.find("value").unwrap());
    editor.set_active_selection(original.clone());

    let mut selections = vec![original.clone()];
    loop {
        let outcome = editor
            .execute(editor_invocation(EditorCommand::ExpandSyntaxSelection))
            .unwrap();
        if matches!(outcome, CommandOutcome::Status(_)) {
            break;
        }
        let next = editor.active_selection();
        assert_ne!(next, *selections.last().unwrap());
        selections.push(next);
    }
    assert!(selections.len() >= 4, "expected nested Rust syntax nodes");

    for expected in selections.iter().rev().skip(1) {
        assert_eq!(
            editor
                .execute(editor_invocation(EditorCommand::ShrinkSyntaxSelection))
                .unwrap(),
            CommandOutcome::Completed
        );
        assert_eq!(&editor.active_selection(), expected);
    }
    assert!(matches!(
        editor
            .execute(editor_invocation(EditorCommand::ShrinkSyntaxSelection))
            .unwrap(),
        CommandOutcome::Status(message) if message == "no syntax expansion to shrink"
    ));
}

#[test]
fn structural_relationship_commands_navigate_real_rust_nodes() {
    let root = IsolatedRoot::new("structural-relations");
    let source = "fn demo() {\n    let first = 1;\n    let second = 2;\n}\n";
    fs::write(root.path().join("relations.rs"), source).unwrap();
    let mut editor = HeadlessEditor::new_in(root.path()).unwrap();
    editor
        .execute(CommandInvocation::open(PathBuf::from("relations.rs")).unwrap())
        .unwrap();
    editor.set_active_selection(Selection::point(source.find("first").unwrap()));
    for _ in 0..8 {
        if selected_text(&editor).trim_start().starts_with("let first") {
            break;
        }
        editor
            .execute(editor_invocation(EditorCommand::ExpandSyntaxSelection))
            .unwrap();
    }
    assert!(selected_text(&editor).trim_start().starts_with("let first"));

    editor
        .execute(editor_invocation(EditorCommand::SelectNextSyntaxSibling))
        .unwrap();
    assert!(
        inclusive_selected_text(&editor)
            .trim_start()
            .starts_with("let second")
    );
    assert_eq!(head_character(&editor), Some(';'));
    editor
        .execute(editor_invocation(
            EditorCommand::SelectPreviousSyntaxSibling,
        ))
        .unwrap();
    assert!(
        inclusive_selected_text(&editor)
            .trim_start()
            .starts_with("let first")
    );
    assert_eq!(head_character(&editor), Some(';'));

    let child = editor.active_selection();
    editor
        .execute(editor_invocation(EditorCommand::SelectSyntaxParent))
        .unwrap();
    assert_ne!(editor.active_selection(), child);
    assert_eq!(head_character(&editor), Some('}'));
    editor
        .execute(editor_invocation(EditorCommand::SelectSyntaxChild))
        .unwrap();
    assert_eq!(editor.active_selection(), child);
    assert_eq!(head_character(&editor), Some(';'));
}

#[test]
fn structural_commands_report_unavailable_without_syntax() {
    let root = IsolatedRoot::new("structural-unavailable");
    let mut editor = HeadlessEditor::with_text_in(root.path(), "plain scratch").unwrap();
    let before = editor.active_selection();
    assert_eq!(
        editor
            .execute(editor_invocation(EditorCommand::ExpandSyntaxSelection))
            .unwrap(),
        CommandOutcome::Unavailable("syntax is unavailable for this buffer".to_owned())
    );
    assert_eq!(editor.active_selection(), before);
    assert_eq!(
        editor
            .execute(editor_invocation(EditorCommand::DocumentOutline))
            .unwrap(),
        CommandOutcome::Unavailable("document outline is unavailable for this buffer".to_owned())
    );
}

#[test]
fn structural_history_is_pane_local_and_invalidated_by_edits() {
    let root = IsolatedRoot::new("structural-pane-history");
    let source = "fn demo() { let value = 1; }\n";
    fs::write(root.path().join("panes.rs"), source).unwrap();
    let mut editor = HeadlessEditor::new_in(root.path()).unwrap();
    editor
        .execute(CommandInvocation::open(PathBuf::from("panes.rs")).unwrap())
        .unwrap();
    let point = Selection::point(source.find("value").unwrap());
    editor.set_active_selection(point.clone());
    editor
        .execute(editor_invocation(EditorCommand::ExpandSyntaxSelection))
        .unwrap();
    let shared_expansion = editor.active_selection();
    editor
        .execute(editor_invocation(EditorCommand::SplitVertical))
        .unwrap();
    assert_eq!(editor.active_selection(), shared_expansion);
    assert_eq!(
        editor
            .execute(editor_invocation(EditorCommand::ShrinkSyntaxSelection))
            .unwrap(),
        CommandOutcome::Status("no syntax expansion to shrink".to_owned())
    );

    editor
        .execute(editor_invocation(EditorCommand::NextWindow))
        .unwrap();
    assert_eq!(editor.active_selection(), shared_expansion);
    editor
        .execute(editor_invocation(EditorCommand::ShrinkSyntaxSelection))
        .unwrap();
    assert_eq!(editor.active_selection(), point);

    editor
        .execute(editor_invocation(EditorCommand::ExpandSyntaxSelection))
        .unwrap();
    let expanded = editor.active_selection();
    editor
        .apply_transaction(Transaction::insert(source.chars().count(), "// edit\n"))
        .unwrap();
    assert!(matches!(
        editor
            .execute(editor_invocation(EditorCommand::ShrinkSyntaxSelection))
            .unwrap(),
        CommandOutcome::Status(message) if message == "no syntax expansion to shrink"
    ));
    assert_eq!(editor.active_selection(), expanded);
}

#[test]
fn structural_expand_after_manual_selection_starts_a_fresh_chain() {
    let root = IsolatedRoot::new("structural-mismatch");
    let source = "fn demo() { let value = 1; }\n";
    fs::write(root.path().join("mismatch.rs"), source).unwrap();
    let mut editor = HeadlessEditor::new_in(root.path()).unwrap();
    editor
        .execute(CommandInvocation::open(PathBuf::from("mismatch.rs")).unwrap())
        .unwrap();
    editor.set_active_selection(Selection::point(source.find("value").unwrap()));
    editor
        .execute(editor_invocation(EditorCommand::ExpandSyntaxSelection))
        .unwrap();
    let changed = Selection::point(source.find("demo").unwrap());
    editor.set_active_selection(changed.clone());
    editor
        .execute(editor_invocation(EditorCommand::ExpandSyntaxSelection))
        .unwrap();

    assert_eq!(
        editor
            .execute(editor_invocation(EditorCommand::ShrinkSyntaxSelection))
            .unwrap(),
        CommandOutcome::Completed
    );
    assert_eq!(editor.active_selection(), changed);
    assert_eq!(
        editor
            .execute(editor_invocation(EditorCommand::ShrinkSyntaxSelection))
            .unwrap(),
        CommandOutcome::Status("no syntax expansion to shrink".to_owned())
    );
}

#[test]
fn structural_history_is_invalidated_by_reload_undo_redo_and_retarget() {
    let root = IsolatedRoot::new("structural-invalidation");
    let source = "fn demo() { let value = 1; }\n";
    fs::write(root.path().join("first.rs"), source).unwrap();
    fs::write(root.path().join("second.rs"), "fn other() {}\n").unwrap();
    let mut editor = HeadlessEditor::new_in(root.path()).unwrap();
    editor
        .execute(CommandInvocation::open(PathBuf::from("first.rs")).unwrap())
        .unwrap();
    let point = Selection::point(source.find("value").unwrap());

    editor.set_active_selection(point.clone());
    editor
        .execute(editor_invocation(EditorCommand::ExpandSyntaxSelection))
        .unwrap();
    editor
        .execute(parse_colon_command("reload").unwrap())
        .unwrap();
    assert_eq!(
        editor
            .execute(editor_invocation(EditorCommand::ShrinkSyntaxSelection))
            .unwrap(),
        CommandOutcome::Status("no syntax expansion to shrink".to_owned())
    );

    editor
        .apply_transaction(Transaction::insert(source.chars().count(), "// edit\n"))
        .unwrap();
    editor.set_active_selection(point.clone());
    editor
        .execute(editor_invocation(EditorCommand::ExpandSyntaxSelection))
        .unwrap();
    editor.undo().unwrap();
    assert_eq!(
        editor
            .execute(editor_invocation(EditorCommand::ShrinkSyntaxSelection))
            .unwrap(),
        CommandOutcome::Status("no syntax expansion to shrink".to_owned())
    );

    editor.set_active_selection(point.clone());
    editor
        .execute(editor_invocation(EditorCommand::ExpandSyntaxSelection))
        .unwrap();
    editor
        .execute(editor_invocation(EditorCommand::Redo))
        .unwrap();
    assert_eq!(
        editor
            .execute(editor_invocation(EditorCommand::ShrinkSyntaxSelection))
            .unwrap(),
        CommandOutcome::Status("no syntax expansion to shrink".to_owned())
    );

    editor.set_active_selection(point);
    editor
        .execute(editor_invocation(EditorCommand::ExpandSyntaxSelection))
        .unwrap();
    editor
        .execute(CommandInvocation::open(PathBuf::from("second.rs")).unwrap())
        .unwrap();
    assert_eq!(
        editor
            .execute(editor_invocation(EditorCommand::ShrinkSyntaxSelection))
            .unwrap(),
        CommandOutcome::Status("no syntax expansion to shrink".to_owned())
    );
}

#[test]
fn delimiter_text_objects_select_nested_pairs_through_headless_commands() {
    let root = IsolatedRoot::new("delimiter-objects");
    let source = "fn demo() { call([1, (2 + 3)]); }\n";
    fs::write(root.path().join("delimiters.rs"), source).unwrap();
    let mut editor = HeadlessEditor::new_in(root.path()).unwrap();
    editor
        .execute(CommandInvocation::open(PathBuf::from("delimiters.rs")).unwrap())
        .unwrap();

    editor.set_active_selection(Selection::point(char_offset(source, "2")));
    editor
        .execute(editor_invocation(EditorCommand::SelectInsideParentheses))
        .unwrap();
    assert_eq!(inclusive_selected_text(&editor), "2 + 3");

    editor.set_active_selection(Selection::point(char_offset(source, "2")));
    editor
        .execute(editor_invocation(EditorCommand::SelectAroundParentheses))
        .unwrap();
    assert_eq!(inclusive_selected_text(&editor), "(2 + 3)");

    editor.set_active_selection(Selection::point(char_offset(source, "2")));
    editor
        .execute(editor_invocation(
            EditorCommand::SelectInsideClosestDelimiter,
        ))
        .unwrap();
    assert_eq!(inclusive_selected_text(&editor), "2 + 3");

    editor
        .execute(editor_invocation(EditorCommand::DeleteSelection))
        .unwrap();
    assert_eq!(editor.active_text(), "fn demo() { call([1, ()]); }\n");

    let empty_pair = editor.active_text().rfind("()").unwrap();
    editor.set_active_selection(Selection::point(empty_pair));
    editor
        .execute(editor_invocation(EditorCommand::SelectInsideParentheses))
        .unwrap();
    editor
        .execute(editor_invocation(EditorCommand::DeleteSelection))
        .unwrap();
    assert_eq!(
        editor.active_text(),
        "fn demo() { call([1, ()]); }\n",
        "an empty inside object must remain an empty operative span"
    );

    editor.undo().unwrap();
    editor.set_active_selection(Selection::point(char_offset(source, "2")));
    editor
        .execute(editor_invocation(EditorCommand::SelectAroundParentheses))
        .unwrap();
    editor
        .execute(editor_invocation(EditorCommand::DeleteSelection))
        .unwrap();
    assert_eq!(editor.active_text(), "fn demo() { call([1, ]); }\n");
}

#[test]
fn syntax_namespace_selections_edit_their_exact_ranges() {
    let root = IsolatedRoot::new("syntax-selection-semantics");
    let source = "fn demo(first: usize) { let value = first + 1; }\n";
    fs::write(root.path().join("semantics.rs"), source).unwrap();

    for command in [
        EditorCommand::ExpandSyntaxSelection,
        EditorCommand::SelectSyntaxParent,
        EditorCommand::SelectSyntaxFunction,
        EditorCommand::SelectInsideSyntaxFunction,
    ] {
        let mut editor = HeadlessEditor::new_in(root.path()).unwrap();
        editor
            .execute(CommandInvocation::open(PathBuf::from("semantics.rs")).unwrap())
            .unwrap();
        editor.set_active_selection(Selection::point(char_offset(source, "value")));
        editor.execute(editor_invocation(command)).unwrap();
        assert_delete_uses_exact_selection(
            &mut editor,
            matches!(
                command,
                EditorCommand::SelectSyntaxParent
                    | EditorCommand::SelectSyntaxFunction
                    | EditorCommand::SelectInsideSyntaxFunction
            ),
        );
    }

    let mut editor = HeadlessEditor::new_in(root.path()).unwrap();
    editor
        .execute(CommandInvocation::open(PathBuf::from("semantics.rs")).unwrap())
        .unwrap();
    let value = char_offset(source, "value");
    editor.set_active_selection(Selection::single(Range::new(value, value + 4)));
    editor
        .execute(editor_invocation(EditorCommand::ExpandSyntaxSelection))
        .unwrap();
    editor
        .execute(editor_invocation(EditorCommand::ShrinkSyntaxSelection))
        .unwrap();
    editor
        .execute(editor_invocation(EditorCommand::DeleteSelection))
        .unwrap();
    assert_eq!(
        editor.active_text(),
        "fn demo(first: usize) { let  = first + 1; }\n"
    );
}

#[test]
fn delimiter_text_objects_work_in_markdown_prose() {
    let root = IsolatedRoot::new("delimiter-objects-markdown");
    let source = "# Notes\n\nCall (alpha [beta]) now.\n";
    fs::write(root.path().join("notes.md"), source).unwrap();

    for (command, expected_selection) in [
        (EditorCommand::SelectInsideParentheses, "alpha [beta]"),
        (EditorCommand::SelectAroundClosestDelimiter, "[beta]"),
    ] {
        let mut editor = HeadlessEditor::new_in(root.path()).unwrap();
        editor
            .execute(CommandInvocation::open(PathBuf::from("notes.md")).unwrap())
            .unwrap();
        editor.set_active_selection(Selection::point(char_offset(source, "beta")));
        editor.execute(editor_invocation(command)).unwrap();
        assert_eq!(inclusive_selected_text(&editor), expected_selection);
        assert_delete_uses_exact_selection(&mut editor, true);
    }
}

#[test]
fn syntax_text_objects_select_rust_functions_classes_and_grouped_parameters() {
    let root = IsolatedRoot::new("syntax-objects-rust");
    let source = "struct Item { value: usize }\nfn greet(first: usize, second: usize) { let local = first; }\n";
    fs::write(root.path().join("objects.rs"), source).unwrap();
    let mut editor = HeadlessEditor::new_in(root.path()).unwrap();
    editor
        .execute(CommandInvocation::open(PathBuf::from("objects.rs")).unwrap())
        .unwrap();

    editor.set_active_selection(Selection::point(char_offset(source, "value")));
    editor
        .execute(editor_invocation(EditorCommand::SelectSyntaxClass))
        .unwrap();
    assert!(inclusive_selected_text(&editor).starts_with("struct Item"));
    editor.set_active_selection(Selection::point(char_offset(source, "value")));
    editor
        .execute(editor_invocation(EditorCommand::SelectInsideSyntaxClass))
        .unwrap();
    assert!(inclusive_selected_text(&editor).contains("value: usize"));
    assert!(!inclusive_selected_text(&editor).contains("struct Item"));

    editor.set_active_selection(Selection::point(char_offset(source, "local")));
    editor
        .execute(editor_invocation(EditorCommand::SelectSyntaxFunction))
        .unwrap();
    assert!(inclusive_selected_text(&editor).starts_with("fn greet"));
    editor.set_active_selection(Selection::point(char_offset(source, "local")));
    editor
        .execute(editor_invocation(EditorCommand::SelectInsideSyntaxFunction))
        .unwrap();
    assert!(inclusive_selected_text(&editor).contains("let local"));
    assert!(!inclusive_selected_text(&editor).contains("fn greet"));

    editor.set_active_selection(Selection::point(char_offset(source, "first")));
    editor
        .execute(editor_invocation(EditorCommand::SelectSyntaxParameter))
        .unwrap();
    assert_eq!(inclusive_selection_pieces(&editor), ["first: usize", ","]);
}

#[test]
fn syntax_grouped_parameter_selection_does_not_fill_uncaptured_gaps() {
    let root = IsolatedRoot::new("syntax-object-gaps");
    let source = "fn greet(first: usize , second: usize) {}\n";
    fs::write(root.path().join("gaps.rs"), source).unwrap();
    let mut editor = HeadlessEditor::new_in(root.path()).unwrap();
    editor
        .execute(CommandInvocation::open(PathBuf::from("gaps.rs")).unwrap())
        .unwrap();
    let gap_from = char_offset(source, " ,");
    let comma = char_offset(source, ",");

    for selection in [
        Selection::single(Range::new(gap_from, comma)),
        Selection::single(Range::new(char_offset(source, "first"), comma + 1)),
    ] {
        editor.set_active_selection(selection.clone());
        assert_eq!(
            editor
                .execute(editor_invocation(EditorCommand::SelectSyntaxParameter))
                .unwrap(),
            CommandOutcome::Status("no enclosing parameter around text object".to_owned())
        );
        assert_eq!(editor.active_selection(), selection);
    }
}

#[test]
fn syntax_text_objects_preserve_unicode_reversed_python_multiselections() {
    let root = IsolatedRoot::new("syntax-objects-python");
    let source = "class Café:\n    def привет(имя: str, суффикс: str):\n        return имя\n";
    fs::write(root.path().join("objects.py"), source).unwrap();
    let mut editor = HeadlessEditor::new_in(root.path()).unwrap();
    editor
        .execute(CommandInvocation::open(PathBuf::from("objects.py")).unwrap())
        .unwrap();
    let first = char_offset(source, "имя: str");
    let second = char_offset(source, "суффикс");
    editor.set_active_selection(Selection::new(
        vec![Range::new(first + 1, first), Range::new(second + 1, second)],
        1,
    ));

    editor
        .execute(editor_invocation(
            EditorCommand::SelectInsideSyntaxParameter,
        ))
        .unwrap();
    let selection = editor.active_selection();
    assert_eq!(selection.primary_index(), 1);
    assert!(
        selection
            .ranges()
            .iter()
            .all(|range| range.anchor > range.head)
    );
    assert_eq!(
        inclusive_selection_pieces(&editor),
        ["имя: str", "суффикс: str"]
    );

    editor.set_active_selection(Selection::point(char_offset(source, "return")));
    editor
        .execute(editor_invocation(EditorCommand::SelectSyntaxFunction))
        .unwrap();
    assert!(inclusive_selected_text(&editor).contains("def привет"));
    editor.set_active_selection(Selection::point(char_offset(source, "return")));
    editor
        .execute(editor_invocation(EditorCommand::SelectSyntaxClass))
        .unwrap();
    assert!(inclusive_selected_text(&editor).starts_with("class Café"));
}

#[test]
fn syntax_text_objects_work_in_injected_markdown_and_malformed_source_is_safe() {
    let root = IsolatedRoot::new("syntax-objects-markdown");
    let markdown = "# Notes\n\n```rust\nfn greet(name: usize) { let value = name; }\n```\n";
    fs::write(root.path().join("notes.md"), markdown).unwrap();
    fs::write(root.path().join("broken.rs"), "fn broken( {\n").unwrap();
    let mut editor = HeadlessEditor::new_in(root.path()).unwrap();
    editor
        .execute(CommandInvocation::open(PathBuf::from("notes.md")).unwrap())
        .unwrap();
    editor.set_active_selection(Selection::point(char_offset(markdown, "value")));
    let outcome = editor
        .execute(editor_invocation(EditorCommand::SelectSyntaxFunction))
        .unwrap();
    assert!(
        matches!(&outcome, CommandOutcome::Unavailable(message) if message.contains("does not support")),
        "pinned injected Rust parsing must degrade explicitly: {outcome:?}"
    );

    editor
        .execute(CommandInvocation::open(PathBuf::from("broken.rs")).unwrap())
        .unwrap();
    let outcome = editor
        .execute(editor_invocation(EditorCommand::SelectSyntaxFunction))
        .unwrap();
    assert!(matches!(
        outcome,
        CommandOutcome::Completed | CommandOutcome::Status(_) | CommandOutcome::Unavailable(_)
    ));
}

#[test]
fn syntax_text_object_capability_failures_are_typed_unavailable() {
    let root = IsolatedRoot::new("syntax-object-unavailable");
    fs::write(root.path().join("unsupported.swift"), "func greet() {}\n").unwrap();
    let mut editor = HeadlessEditor::new_in(root.path()).unwrap();
    editor
        .execute(CommandInvocation::open(PathBuf::from("unsupported.swift")).unwrap())
        .unwrap();
    let outcome = editor
        .execute(editor_invocation(EditorCommand::SelectSyntaxFunction))
        .unwrap();
    assert!(matches!(
        outcome,
        CommandOutcome::Unavailable(message) if message.contains("does not support")
    ));

    let mut scratch = HeadlessEditor::with_text_in(root.path(), "plain").unwrap();
    assert_eq!(
        scratch
            .execute(editor_invocation(EditorCommand::SelectSyntaxFunction))
            .unwrap(),
        CommandOutcome::Unavailable("syntax is unavailable for this buffer".to_owned())
    );
}

#[test]
fn syntax_object_navigation_supports_modes_counts_and_each_object_kind() {
    let root = IsolatedRoot::new("syntax-object-navigation");
    let source = "struct First;\nstruct Second;\nfn first(a: usize, b: usize) {}\nfn second(c: usize) {}\nfn third() {}\n";
    fs::write(root.path().join("navigation.rs"), source).unwrap();
    let mut editor = HeadlessEditor::new_in(root.path()).unwrap();
    editor
        .execute(CommandInvocation::open(PathBuf::from("navigation.rs")).unwrap())
        .unwrap();
    let first_fn = char_offset(source, "fn first");
    let second_fn = char_offset(source, "fn second");
    let third_fn = char_offset(source, "fn third");
    let two = CommandExecutionContext::resolved(NonZeroUsize::new(2).unwrap(), None);

    editor.set_active_selection(Selection::point(first_fn));
    editor
        .execute(CommandInvocation::editor(EditorCommand::GotoNextSyntaxFunction, two).unwrap())
        .unwrap();
    assert_eq!(editor.active_selection(), Selection::point(third_fn));
    editor
        .execute(CommandInvocation::editor(EditorCommand::GotoPreviousSyntaxFunction, two).unwrap())
        .unwrap();
    assert_eq!(editor.active_selection(), Selection::point(first_fn));

    editor
        .execute(editor_invocation(EditorCommand::EnterSelectMode))
        .unwrap();
    editor
        .execute(editor_invocation(EditorCommand::GotoNextSyntaxFunction))
        .unwrap();
    assert_eq!(
        editor.active_selection(),
        Selection::single(Range::new(first_fn, second_fn))
    );

    editor
        .execute(editor_invocation(EditorCommand::EnterNormalMode))
        .unwrap();
    editor.set_active_selection(Selection::point(char_offset(source, "struct First")));
    editor
        .execute(editor_invocation(EditorCommand::GotoNextSyntaxClass))
        .unwrap();
    assert_eq!(
        editor.active_selection(),
        Selection::point(char_offset(source, "struct Second"))
    );
    editor
        .execute(editor_invocation(EditorCommand::GotoPreviousSyntaxClass))
        .unwrap();
    assert_eq!(
        editor.active_selection(),
        Selection::point(char_offset(source, "struct First"))
    );
    editor.set_active_selection(Selection::point(char_offset(source, "a: usize")));
    editor
        .execute(editor_invocation(EditorCommand::GotoNextSyntaxParameter))
        .unwrap();
    assert_eq!(
        editor.active_selection(),
        Selection::point(char_offset(source, "b: usize"))
    );
    editor
        .execute(editor_invocation(
            EditorCommand::GotoPreviousSyntaxParameter,
        ))
        .unwrap();
    assert_eq!(
        editor.active_selection(),
        Selection::point(char_offset(source, "a: usize"))
    );

    editor.set_active_selection(Selection::new(
        vec![
            Selection::point(first_fn).primary(),
            Selection::point(third_fn).primary(),
        ],
        0,
    ));
    editor
        .execute(editor_invocation(EditorCommand::GotoNextSyntaxFunction))
        .unwrap();
    assert_eq!(
        editor.active_selection(),
        Selection::new(vec![Range::point(second_fn), Range::point(third_fn)], 0)
    );
    assert!(matches!(
        editor
            .execute(editor_invocation(EditorCommand::GotoNextSyntaxFunction))
            .unwrap(),
        CommandOutcome::Completed
    ));
    assert!(matches!(
        editor
            .execute(editor_invocation(EditorCommand::GotoNextSyntaxFunction))
            .unwrap(),
        CommandOutcome::Status(message) if message == "no next syntax function"
    ));

    assert!(
        CommandInvocation::editor(EditorCommand::SelectSyntaxFunction, two).is_err(),
        "select-object commands must reject counts"
    );
}
