// SPDX-License-Identifier: MPL-2.0

use std::{fs, path::PathBuf, time::Duration};

use runyte::{
    command::parse_colon_command,
    headless::HeadlessEditor,
    snapshot::{SnapshotRow, TextRunKind},
    text::Transaction,
};

struct TempDir(PathBuf);

impl TempDir {
    fn new(label: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "runyte-background-syntax-{label}-{}-{}",
            std::process::id(),
            std::thread::current().name().unwrap_or("test")
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn highlighted(editor: &mut HeadlessEditor) -> bool {
    editor.snapshot(100, 20).panes.iter().any(|pane| {
        pane.rows.iter().any(|row| {
            matches!(row, SnapshotRow::Text(row) if row.runs.iter().any(|run| {
                matches!(run.kind, TextRunKind::Text { scope: Some(_), .. })
            }))
        })
    })
}

fn rust_editor(label: &str) -> (TempDir, HeadlessEditor) {
    let root = TempDir::new(label);
    let path = root.0.join("sample.rs");
    fs::write(
        &path,
        "fn main() {\n    let answer = 41;\n    println!(\"{answer}\");\n}\n",
    )
    .unwrap();
    let mut editor = HeadlessEditor::new_in(&root.0).unwrap();
    editor
        .execute(parse_colon_command(&format!("open {}", path.display())).unwrap())
        .unwrap();
    (root, editor)
}

#[tokio::test(flavor = "multi_thread")]
async fn stale_tree_exposes_translated_spans_but_no_structure_until_drain() {
    let (_root, mut editor) = rust_editor("stale-boundary");
    assert!(highlighted(&mut editor));
    assert!(editor.active_outline().unwrap().is_some());
    let mut events = editor.enable_background_syntax();

    editor
        .apply_transaction(Transaction::insert(0, "// pending\n"))
        .unwrap();
    assert!(editor.has_pending_syntax());
    assert!(
        editor.active_outline().unwrap().is_none(),
        "structural queries must not see a stale tree"
    );
    assert!(
        highlighted(&mut editor),
        "translated spans should keep colours visible while parsing"
    );

    let event = tokio::time::timeout(Duration::from_secs(5), events.recv())
        .await
        .expect("background parse timed out")
        .expect("background worker stopped");
    let _snapshot = editor.snapshot(100, 20);
    assert!(
        editor.active_outline().unwrap().is_none(),
        "preparing a frame must not apply an undrained syntax event"
    );

    assert!(editor.apply_syntax_event(event));
    assert!(!editor.has_pending_syntax());
    assert!(editor.active_outline().unwrap().is_some());
}

#[tokio::test(flavor = "multi_thread")]
async fn late_tree_is_rejected_and_the_latest_coalesced_revision_applies() {
    let (_root, mut editor) = rust_editor("late-result");
    let base_revision = editor.active_outline().unwrap().unwrap().revision;
    let mut events = editor.enable_background_syntax();

    editor
        .apply_transaction(Transaction::insert(0, "// first\n"))
        .unwrap();
    let first = tokio::time::timeout(Duration::from_secs(5), events.recv())
        .await
        .expect("first parse timed out")
        .expect("background worker stopped");

    editor
        .apply_transaction(Transaction::insert(0, "// second\n"))
        .unwrap();
    assert!(
        !editor.apply_syntax_event(first),
        "a tree parsed for the previous text revision must be rejected"
    );
    for _ in 0..8 {
        editor
            .apply_transaction(Transaction::insert(0, "// newer\n"))
            .unwrap();
    }

    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let event = events.recv().await.expect("background worker stopped");
            if editor.apply_syntax_event(event) {
                break;
            }
        }
    })
    .await
    .expect("coalesced parse timed out");
    assert!(!editor.has_pending_syntax());
    assert!(highlighted(&mut editor));
    assert_eq!(
        editor.active_outline().unwrap().unwrap().revision.get(),
        base_revision.get() + 1,
        "the typing burst should be coalesced into one incremental update"
    );
}

#[tokio::test]
async fn initial_document_frame_and_edits_do_not_require_a_syntax_tree() {
    let root = TempDir::new("initial-frame");
    let path = root.0.join("initial.rs");
    let original = "fn main() {\n    let café = 41;\n}\n";
    fs::write(&path, original).unwrap();
    let mut editor = HeadlessEditor::new_deferred_in(&root.0).unwrap();
    editor
        .execute(parse_colon_command(&format!("open {}", path.display())).unwrap())
        .unwrap();
    assert_eq!(editor.active_text(), original);
    assert!(editor.has_pending_syntax());
    assert!(!highlighted(&mut editor));
    let frame = editor.snapshot(100, 20);
    assert!(frame.panes.iter().any(|pane| pane.rows.iter().any(|row| {
        matches!(row, SnapshotRow::Text(row) if row.runs.iter().map(|run| run.text.as_str()).collect::<String>().contains("fn main"))
    })));
    editor
        .apply_transaction(Transaction::insert(0, "// α\n"))
        .unwrap();
    let expected = format!("// α\n{original}");
    assert_eq!(editor.active_text(), expected);
    assert!(editor.active_outline().unwrap().is_none());
    let mut events = editor.enable_background_syntax();
    let event = tokio::time::timeout(Duration::from_secs(5), events.recv())
        .await
        .unwrap()
        .unwrap();
    let selection = editor.active_selection();
    assert!(
        !highlighted(&mut editor),
        "an undrained result cannot affect frame preparation"
    );
    assert!(editor.apply_syntax_event(event));
    assert!(highlighted(&mut editor));
    assert_eq!(editor.active_selection(), selection);
    assert_eq!(editor.active_text(), expected);
    editor.undo().unwrap();
    assert_eq!(editor.active_text(), original);
}
