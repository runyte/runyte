// SPDX-License-Identifier: MPL-2.0

//! Performance limits Runyte has to meet on large documents.
//!
//! These are budgets, not measurements: each one is far above what the editor
//! currently needs, because the point is to catch a change that makes an
//! operation pathological rather than to record today's numbers. A regression
//! here is a stall a person would feel, not a few percent.
//!
//! Two shapes of large file are covered, because they fail in different ways.
//! A document with very many rows stresses everything that walks lines; a
//! minified document, where one line is the whole file, stresses everything
//! that works from the start of a line. Both are checked with soft wrap on and
//! off, since wrapping is measured per logical line and is the part most
//! easily made quadratic.
//!
//! Wall-clock budgets are meaningful only when these heavyweight cases do not
//! compete with one another. They are ignored by the ordinary debug suite and
//! run serially in release mode by CI. The equivalent local command is:
//! `cargo test --release --test performance -- --ignored --test-threads=1`.

use std::{
    fs,
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
    time::{Duration, Instant},
};

use runyte::{
    app::App,
    buffer::SOFT_WRAP_LINE_LIMIT,
    command::{CommandExecutionContext, CommandInvocation, EditorCommand, parse_colon_command},
    config::Config,
    file_picker::{CONTENT_ENTRY_LIMIT, FileHits, FilePicker, scan_content},
    finder::{FinderMode, ResourceFinder, ResourceItem, ResourceKind, ResourceTarget},
    headless::HeadlessEditor,
    input::{KeyCode, KeyStroke, Modifiers},
    selection::Selection,
    snapshot::{EditorSnapshot, SnapshotRow, TextRunKind},
    syntax::SyntaxEvents,
    text::{Text, Transaction},
    wrap,
};

/// Whether any pane shows a wrapped row, which is how a soft-wrapped document
/// is told apart from one shown a line to a row.
fn has_continuation(snapshot: &EditorSnapshot) -> bool {
    snapshot.panes.iter().any(|pane| {
        pane.rows
            .iter()
            .any(|row| matches!(row, SnapshotRow::Text(row) if row.continuation))
    })
}

/// Whether any visible run carries a syntax scope, which is how a highlighted
/// document is told apart from one shown as plain text.
fn has_syntax_scope(snapshot: &EditorSnapshot) -> bool {
    snapshot.panes.iter().any(|pane| {
        pane.rows.iter().any(|row| {
            let SnapshotRow::Text(row) = row else {
                return false;
            };
            row.runs
                .iter()
                .any(|run| matches!(run.kind, TextRunKind::Text { scope: Some(_), .. }))
        })
    })
}

/// One frame at 60Hz. Every redraw budget here is this, because a redraw that
/// misses it is a redraw the person sees miss it.
const FRAME: Duration = Duration::from_millis(16);

/// What a document Runyte chooses to soft-wrap has to redraw within.
///
/// Wrapping a very long line is linear and unavoidably costly, so the line
/// limit is set where a frame would reach about a second. This is the other
/// half of that promise: anything under the limit, and therefore still
/// wrapped, has to stay inside it. Between one frame and this ceiling the
/// editor is progressively slower to scroll but still usable, which is the
/// trade the limit deliberately makes.
const WRAPPED_FRAME_CEILING: Duration = Duration::from_secs(1);

/// Budgets describe an optimized build, the only one whose numbers say
/// anything about what a person feels. A debug build does the same work an
/// order of magnitude slower, so the limits are relaxed there rather than
/// skipped: `cargo test` still catches anything pathological, and
/// `cargo test --release` holds the real line.
fn budget(release: Duration) -> Duration {
    if cfg!(debug_assertions) {
        release * 25
    } else {
        release
    }
}

#[track_caller]
fn within(label: &str, elapsed: Duration, limit: Duration) {
    assert!(
        elapsed <= limit,
        "{label} took {elapsed:?}, over its {limit:?} budget"
    );
}

fn fixture_root() -> &'static PathBuf {
    static ROOT: OnceLock<PathBuf> = OnceLock::new();
    ROOT.get_or_init(|| {
        // Deliberately stable rather than per-run: these fixtures are tens of
        // megabytes, and a fresh directory each run would leave a pile of them
        // behind in the temporary directory.
        let root = std::env::temp_dir().join("runyte-performance-fixtures");
        fs::create_dir_all(&root).unwrap();
        root
    })
}

/// Builds a fixture once and reuses it afterwards, so the cost of writing tens
/// of megabytes is charged neither to every test nor to every run.
///
/// The content is written under a name unique to this process and then renamed
/// into place, because the directory is shared: two runs at once must not read
/// a file the other is still writing.
fn fixture(name: &str, build: fn() -> String) -> PathBuf {
    static BUILD: Mutex<()> = Mutex::new(());
    let _build = BUILD.lock().unwrap();
    let path = fixture_root().join(name);
    if !path.exists() {
        let pending = fixture_root().join(format!("{name}.{}.pending", std::process::id()));
        fs::write(&pending, build()).unwrap();
        fs::rename(&pending, &path).unwrap();
    }
    path
}

/// A million rows of ordinary text, around 45MB.
fn million_rows() -> PathBuf {
    fixture("million_rows.txt", || {
        let mut text = String::with_capacity(46_000_000);
        for index in 0..1_000_000 {
            text.push_str(&format!("line {index} of the document with some content\n"));
        }
        text
    })
}

/// Rows behind a grammar, large enough that synchronous reparsing is visible.
fn highlighted_rows() -> PathBuf {
    fixture("highlighted_rows.json", || {
        let mut text = String::from("[\n");
        for index in 0..150_000 {
            text.push_str(&format!(
                "  {{\"id\": {index}, \"name\": \"item-{index}\"}},\n"
            ));
        }
        text.push_str("  null\n]\n");
        text
    })
}

/// Rows past the former 200,000-line refusal.
fn past_the_old_syntax_line_limit() -> PathBuf {
    fixture("over_syntax_limit.json", || {
        let mut text = String::from("[\n");
        for index in 0..250_000 {
            text.push_str(&format!(
                "  {{\"id\": {index}, \"name\": \"item-{index}\"}},\n"
            ));
        }
        text.push_str("  null\n]\n");
        text
    })
}

/// One line holding the whole document, the shape of a minified file.
fn minified_json() -> PathBuf {
    fixture("minified.json", || {
        let mut text = String::from("{\"items\":[");
        for index in 0..52_000 {
            if index > 0 {
                text.push(',');
            }
            text.push_str(&format!("{{\"id\":{index},\"name\":\"item-{index}\"}}"));
        }
        text.push_str("]}");
        text
    })
}

/// One line past [`runyte::buffer::SOFT_WRAP_LINE_LIMIT`], so the point where
/// wrapping is withheld is tested where it actually sits rather than in the
/// abstract. Large, and reused across runs for that reason.
fn over_the_wrap_limit() -> PathBuf {
    fixture("over_wrap_limit.txt", || {
        "abcdefghij ".repeat(SOFT_WRAP_LINE_LIMIT / 11 + 100_000)
    })
}

/// A single line past the former 8 MB syntax refusal.
fn past_the_old_syntax_byte_limit() -> PathBuf {
    fixture("over_syntax_bytes.json", || {
        let mut text = String::from("{\"items\":[");
        while text.len() < 9 * 1024 * 1024 {
            if text.len() > 10 {
                text.push(',');
            }
            let index = text.len();
            text.push_str(&format!("{{\"id\":{index},\"name\":\"item-{index}\"}}"));
        }
        text.push_str("]}");
        text
    })
}

/// Prose with lines long enough to wrap but short enough to stay wrappable.
fn wrapping_prose() -> PathBuf {
    fixture("prose.md", || {
        let paragraph = "Wrapped prose that runs well past the width of any pane it is shown in \
             and therefore has to be broken into several screen rows to be read. ";
        let mut text = String::new();
        for _ in 0..20_000 {
            text.push_str(paragraph);
            text.push('\n');
        }
        text
    })
}

fn editor_at(path: &Path, soft_wrap: bool) -> HeadlessEditor {
    let mut editor = HeadlessEditor::new_in(fixture_root()).unwrap();
    if soft_wrap {
        editor
            .execute(
                CommandInvocation::editor(
                    EditorCommand::ToggleSoftWrap,
                    CommandExecutionContext::default(),
                )
                .unwrap(),
            )
            .unwrap();
    }
    editor
        .execute(parse_colon_command(&format!("open {}", path.display())).unwrap())
        .unwrap();
    editor
}

/// The slowest of several redraws, so a budget is not met by one lucky frame.
fn slowest_frame(editor: &mut HeadlessEditor) -> Duration {
    (0..3)
        .map(|_| {
            let start = Instant::now();
            editor.snapshot(120, 40);
            start.elapsed()
        })
        .max()
        .unwrap()
}

/// Wrapping has to be linear in the length of the line.
///
/// Deriving the display cell of each break by rescanning the line from its
/// start once made this quadratic, which is what turned a one-line file into a
/// hang. The budget is generous by two orders of magnitude on purpose: a
/// linear implementation finishes this in single-digit milliseconds, and a
/// quadratic one takes minutes, so nothing in between needs deciding.
#[test]
#[ignore = "run serially in the release performance job"]
fn wrapping_a_very_long_line_is_linear_in_its_length() {
    let line: String = "abcdefghij ".repeat(200_000);
    assert!(line.chars().count() > 2_000_000);
    let start = Instant::now();
    let spans = wrap::segments(&line, 100, 4);
    let elapsed = start.elapsed();
    assert!(spans.len() > 20_000);
    within(
        "wrapping a 2,200,000 character line",
        elapsed,
        budget(Duration::from_millis(250)),
    );
}

#[test]
#[ignore = "run serially in the release performance job"]
fn a_million_row_file_opens_and_redraws_within_budget() {
    let path = million_rows();
    for soft_wrap in [false, true] {
        let start = Instant::now();
        let mut editor = editor_at(&path, soft_wrap);
        within(
            &format!("opening a million rows (soft wrap {soft_wrap})"),
            start.elapsed(),
            budget(Duration::from_millis(1500)),
        );
        within(
            &format!("redrawing a million rows (soft wrap {soft_wrap})"),
            slowest_frame(&mut editor),
            budget(FRAME),
        );
    }
}

/// Moving the caret far from the viewport must not cost a walk of the document.
///
/// Measuring the visual gap between the viewport and the caret means wrapping
/// every line in between, so the count is taken only as far as one screen.
#[test]
#[ignore = "run serially in the release performance job"]
fn a_caret_far_from_the_viewport_redraws_within_budget() {
    let path = million_rows();
    let text_len = fs::read_to_string(&path).unwrap().chars().count();
    for soft_wrap in [false, true] {
        let mut editor = editor_at(&path, soft_wrap);
        editor.snapshot(120, 40);
        editor.set_active_selection(Selection::point(text_len * 9 / 10));
        within(
            &format!("redrawing after a far jump (soft wrap {soft_wrap})"),
            slowest_frame(&mut editor),
            budget(FRAME),
        );
    }
}

/// Highlighting has to be paid for the rows on screen, not for the document.
#[test]
#[ignore = "run serially in the release performance job"]
fn a_large_highlighted_file_opens_and_redraws_within_budget() {
    let path = highlighted_rows();
    for soft_wrap in [false, true] {
        let start = Instant::now();
        let mut editor = editor_at(&path, soft_wrap);
        within(
            &format!("opening 150,000 highlighted rows (soft wrap {soft_wrap})"),
            start.elapsed(),
            budget(Duration::from_millis(1500)),
        );
        within(
            &format!("redrawing 150,000 highlighted rows (soft wrap {soft_wrap})"),
            slowest_frame(&mut editor),
            budget(FRAME),
        );
    }
}

/// The reported case: a megabyte-and-a-half of minified JSON on one line.
///
/// Unwrapped it has to meet the ordinary frame budget. Wrapped it only has to
/// meet the promise the line limit makes — see [`WRAPPED_FRAME_CEILING`] — and
/// it is a long way inside it, at roughly 17ms.
#[test]
#[ignore = "run serially in the release performance job"]
fn a_minified_single_line_file_opens_and_redraws_within_budget() {
    let path = minified_json();
    assert!(fs::metadata(&path).unwrap().len() > 1_500_000);
    for soft_wrap in [false, true] {
        let start = Instant::now();
        let mut editor = editor_at(&path, soft_wrap);
        within(
            &format!("opening a minified file (soft wrap {soft_wrap})"),
            start.elapsed(),
            budget(Duration::from_millis(750)),
        );
        within(
            &format!("redrawing a minified file (soft wrap {soft_wrap})"),
            slowest_frame(&mut editor),
            budget(if soft_wrap {
                WRAPPED_FRAME_CEILING
            } else {
                FRAME
            }),
        );
    }
}

/// The delay between a keystroke and the character appearing: the edit plus
/// the frame that shows it.
fn slowest_keystroke(editor: &mut HeadlessEditor, at: usize) -> Duration {
    (0..5)
        .map(|index| {
            let start = Instant::now();
            editor
                .apply_transaction(Transaction::insert(at + index, "x"))
                .unwrap();
            editor.snapshot(120, 40);
            start.elapsed()
        })
        .max()
        .unwrap()
}

async fn apply_finished_syntax(editor: &mut HeadlessEditor, events: &mut SyntaxEvents) {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let event = events
                .recv()
                .await
                .expect("background syntax worker stopped");
            if editor.apply_syntax_event(event) {
                break;
            }
        }
    })
    .await
    .expect("background syntax parse timed out");
}

/// Typing into a document with no grammar behind it costs the edit, not the
/// document, however large it is.
#[test]
#[ignore = "run serially in the release performance job"]
fn typing_into_a_large_plain_file_stays_within_budget() {
    let mut editor = editor_at(&million_rows(), true);
    editor.snapshot(120, 40);
    within(
        "typing into a million rows",
        slowest_keystroke(&mut editor, 0),
        budget(FRAME),
    );
}

/// Typing into a large highlighted document queues a background reparse, so
/// the keystroke itself costs nothing extra.
///
/// The reparse Tree-sitter does after an edit is incremental, but its cost
/// grows with the size of the document rather than with the size of the edit:
/// a flat structure gives the root node one child per element, and an edit
/// anywhere rebuilds that list. Measured on this fixture it is roughly linear
/// — about 6ms at 25,000 rows and about 96ms at 200,000 — which is why it
/// cannot run on the keystroke that caused it.
///
/// The retained tree translates viewport spans through the pending edits, so
/// the document remains coloured for the whole burst.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "run serially in the release performance job"]
async fn typing_into_a_large_highlighted_file_keeps_colours_during_reparse() {
    let path = highlighted_rows();
    let mut editor = editor_at(&path, true);
    assert!(
        has_syntax_scope(&editor.snapshot(120, 40)),
        "the document should start out highlighted"
    );
    let mut events = editor.enable_background_syntax();

    editor
        .execute(
            CommandInvocation::editor(
                EditorCommand::EnterInsertMode,
                CommandExecutionContext::default(),
            )
            .unwrap(),
        )
        .unwrap();
    // Mid-document, so the edit is a realistic one rather than a caret sitting
    // outside the structure the grammar has parsed.
    let middle = fs::metadata(&path).unwrap().len() as usize / 2;
    within(
        "typing into a large highlighted file",
        slowest_keystroke(&mut editor, middle),
        budget(FRAME),
    );
    assert!(
        editor.has_pending_syntax(),
        "a burst in a large document should queue a reparse"
    );
    assert!(
        has_syntax_scope(&editor.snapshot(120, 40)),
        "translated spans should keep colours during the burst"
    );

    apply_finished_syntax(&mut editor, &mut events).await;
    assert!(!editor.has_pending_syntax());
    assert!(
        has_syntax_scope(&editor.snapshot(120, 40)),
        "the completed tree should remain highlighted"
    );
}

/// A minified document takes the same background path despite having one line,
/// and documents past the former byte refusal remain highlighted.
///
/// Reparsing costs what a document holds rather than how it is broken up, so a
/// one-line fixture verifies both the asynchronous edit path and removal of
/// the former byte refusal.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "run serially in the release performance job"]
async fn minified_documents_reparse_in_background_past_the_old_byte_limit() {
    let path = minified_json();
    assert_eq!(
        Text::from_str(&fs::read_to_string(&path).unwrap()).len_lines(),
        1,
        "the fixture has to be a single line for this to test anything"
    );

    let mut editor = editor_at(&path, false);
    assert!(has_syntax_scope(&editor.snapshot(120, 40)));
    let mut events = editor.enable_background_syntax();
    editor
        .execute(
            CommandInvocation::editor(
                EditorCommand::EnterInsertMode,
                CommandExecutionContext::default(),
            )
            .unwrap(),
        )
        .unwrap();
    within(
        "typing into a minified document",
        slowest_keystroke(&mut editor, 100),
        budget(FRAME),
    );
    assert!(
        editor.has_pending_syntax(),
        "a minified document should queue its reparse"
    );
    assert!(has_syntax_scope(&editor.snapshot(120, 40)));
    apply_finished_syntax(&mut editor, &mut events).await;

    let path = past_the_old_syntax_byte_limit();
    let mut oversized = HeadlessEditor::new_in(fixture_root()).unwrap();
    oversized
        .execute(parse_colon_command(&format!("open {}", path.display())).unwrap())
        .unwrap();
    assert!(
        has_syntax_scope(&oversized.snapshot(120, 40)),
        "a single line past the former byte limit should be highlighted"
    );
}

/// A discrete edit only queues parser work and stays inside one frame.
///
/// This isolates syntax dispatch from wrapping. A multi-megabyte logical line
/// has its own deliberate wrapped-frame ceiling above; charging that redraw to
/// this assertion would test the wrapping policy rather than parser latency.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "run serially in the release performance job"]
async fn a_discrete_edit_in_a_large_highlighted_file_stays_under_its_ceiling() {
    const REPARSE_CEILING: Duration = FRAME;
    for (label, path) in [
        ("a large highlighted file", highlighted_rows()),
        ("a minified file", minified_json()),
    ] {
        let mut editor = editor_at(&path, false);
        editor.snapshot(120, 40);
        let _events = editor.enable_background_syntax();
        let middle = fs::metadata(&path).unwrap().len() as usize / 2;
        within(
            &format!("editing {label}"),
            slowest_keystroke(&mut editor, middle),
            budget(REPARSE_CEILING),
        );
        assert!(
            editor.has_pending_syntax(),
            "a discrete edit should queue parser work"
        );
    }
}

/// Wrapping stays on, and stays fast, for the documents it is meant for.
#[test]
#[ignore = "run serially in the release performance job"]
fn a_large_wrapped_prose_file_redraws_within_budget() {
    let mut editor = editor_at(&wrapping_prose(), true);
    let snapshot = editor.snapshot(120, 40);
    assert!(
        has_continuation(&snapshot),
        "prose within the line limit should still be soft-wrapped"
    );
    within(
        "redrawing wrapped prose",
        slowest_frame(&mut editor),
        budget(FRAME),
    );
}

/// Soft wrap is withheld only where wrapping a frame would take about a
/// second, and kept everywhere below that.
///
/// The check is on the measured length of the longest line rather than on the
/// file's size or name, so a large file made of ordinary lines is unaffected,
/// and so is a minified file of a few megabytes.
#[test]
#[ignore = "run serially in the release performance job"]
fn soft_wrap_is_withheld_only_from_a_line_that_takes_about_a_second() {
    let mut prose = editor_at(&wrapping_prose(), true);
    assert!(
        has_continuation(&prose.snapshot(120, 40)),
        "a document of ordinary lines should keep soft wrap"
    );

    let mut minified = editor_at(&minified_json(), true);
    assert!(
        has_continuation(&minified.snapshot(120, 40)),
        "a few megabytes on one line is still cheap enough to wrap"
    );

    let mut huge = editor_at(&over_the_wrap_limit(), true);
    let snapshot = huge.snapshot(120, 40);
    assert!(
        !has_continuation(&snapshot),
        "a line past the limit should be shown unwrapped"
    );
    // Withholding wrapping is what makes this document usable at all, so the
    // ordinary frame budget applies to it rather than the wrapped ceiling.
    within(
        "redrawing a line past the wrap limit",
        slowest_frame(&mut huge),
        budget(FRAME),
    );
}

/// A document past the former line refusal is highlighted and its edits stay
/// responsive because reparsing is background work.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "run serially in the release performance job"]
async fn syntax_highlighting_continues_past_the_old_line_limit() {
    let mut highlighted = editor_at(&highlighted_rows(), false);
    assert!(
        has_syntax_scope(&highlighted.snapshot(120, 40)),
        "a document under the line limit should still be highlighted"
    );

    let path = past_the_old_syntax_line_limit();
    let mut oversized = HeadlessEditor::new_in(fixture_root()).unwrap();
    oversized
        .execute(parse_colon_command(&format!("open {}", path.display())).unwrap())
        .unwrap();
    assert!(
        has_syntax_scope(&oversized.snapshot(120, 40)),
        "a document over the former line limit should be highlighted"
    );

    within(
        "redrawing a document over the former syntax limit",
        slowest_frame(&mut oversized),
        budget(FRAME),
    );
    let _events = oversized.enable_background_syntax();
    within(
        "typing into a document over the former syntax limit",
        slowest_keystroke(&mut oversized, 0),
        budget(FRAME),
    );
}

// -- Fuzzy content search ---------------------------------------------------

/// The needle, and the query that reaches it.
///
/// Deliberately not a word any generated line contains as a subsequence:
/// content search is fuzzy, so a query has to be checked against the filler
/// rather than merely look unlike it.
const GREP_NEEDLE: &str = "call_the_marked_thing(context);";
const GREP_QUERY: &str = "markedthing";

/// A project too large for its lines to all be candidates at once: 600 files
/// over 30 directories, 300,000 lines in all, plus one file of 60,000 lines so
/// the single-large-file path is covered too.
///
/// The needle sits in one file near the end of the walk. The point is not
/// where it is — a filtered scan does not care — but that far more than
/// `CONTENT_ENTRY_LIMIT` lines lie between the root and it, which is what a
/// scan bounded by lines read rather than by matches found could never cross.
fn large_project() -> &'static Path {
    static ROOT: OnceLock<PathBuf> = OnceLock::new();
    ROOT.get_or_init(|| {
        let root = fixture_root().join("large_project");
        if root.exists() {
            return root;
        }
        let pending = fixture_root().join(format!("large_project.{}.pending", std::process::id()));
        let filler = (0..500)
            .map(|line| format!("    let value_{line} = compute(input, {line}) + offset;\n"))
            .collect::<String>();
        for file in 0..600 {
            let directory = pending.join(format!("module{}", file % 30));
            fs::create_dir_all(&directory).unwrap();
            fs::write(directory.join(format!("part{file}.rs")), &filler).unwrap();
        }
        fs::write(
            pending.join("one_large_file.rs"),
            (0..120).map(|_| filler.as_str()).collect::<String>(),
        )
        .unwrap();
        fs::write(
            pending.join("zzz_needle.rs"),
            format!("{filler}{GREP_NEEDLE}\n"),
        )
        .unwrap();
        // Renamed into place so a second run never reads a tree this one is
        // still writing, the same way the single-file fixtures are built.
        if fs::rename(&pending, &root).is_err() {
            fs::remove_dir_all(&pending).ok();
        }
        root
    })
}

/// What a full content scan of a large project has to finish within.
///
/// This reads every tracked file and tests every line, so it is linear in the
/// project and cannot be made instant. The budget is where a person would
/// start to feel the picker lag behind their typing rather than where the scan
/// currently lands, which is several times under it.
const GREP_SCAN_CEILING: Duration = Duration::from_millis(1_500);

/// Content search has to reach a match anywhere in a project, not only in the
/// part of it a scan happened to read first.
///
/// The scan filters as it walks, so its candidate ceiling bounds the matches
/// it collects rather than how far into the project it got. The empty query is
/// the control: it matches everything, so it does still fill the budget and
/// stop early, and that is the state the picker leaves as soon as anything is
/// typed.
#[test]
#[ignore = "run serially in the release performance job"]
fn a_content_scan_finds_a_match_anywhere_in_a_large_project() {
    let root = large_project();
    let state_root = root.join(".runyte");

    let start = Instant::now();
    let (entries, _, limited) = scan_content(root, root, &state_root, false, GREP_QUERY).unwrap();
    let elapsed = start.elapsed();
    assert!(
        !limited,
        "one match in 360,000 lines must not report a truncated scan"
    );
    assert_eq!(
        entries
            .iter()
            .flat_map(|hits| hits.lines.iter().map(|line| (
                hits.path.strip_prefix(root).unwrap().to_path_buf(),
                line.text.clone()
            )))
            .collect::<Vec<_>>(),
        [(PathBuf::from("zzz_needle.rs"), GREP_NEEDLE.to_owned())],
        "the needle is the only line the query matches, and it has to be found"
    );
    within(
        "scanning a large project for one match",
        elapsed,
        budget(GREP_SCAN_CEILING),
    );

    let start = Instant::now();
    let (entries, _, limited) = scan_content(root, root, &state_root, false, "").unwrap();
    assert!(
        limited,
        "a query matching every line has to stop at the cap"
    );
    assert_eq!(
        entries.iter().map(FileHits::len).sum::<usize>(),
        CONTENT_ENTRY_LIMIT,
        "the budget counts matching lines, not the files holding them"
    );
    within(
        "opening content search on a large project",
        start.elapsed(),
        budget(GREP_SCAN_CEILING),
    );
}

/// One complete content-ranking pass has to stay bounded.
///
/// The terminal editor queues this pass on the background file ranker, so it
/// is no longer work a keystroke waits for. The candidate ceiling remains a
/// useful bound on how soon the current revision's rows arrive. Every
/// candidate here matches, which is the expensive case rather than the
/// typical one. The synchronous `FilePicker` seam used by embedders exercises
/// the same scoring kernel without including scanner I/O.
#[test]
#[ignore = "run serially in the release performance job"]
fn ranking_a_full_content_budget_stays_within_a_frame() {
    let root = large_project();
    let (entries, _, limited) =
        scan_content(root, root, &root.join(".runyte"), false, "value").unwrap();
    assert!(limited, "the fixture has to fill the ranking budget");
    let mut picker = FilePicker::grep(1, root.to_path_buf());
    picker.add_content(entries);
    assert_eq!(picker.entries.len(), CONTENT_ENTRY_LIMIT);

    picker.insert_query_text("comput");
    assert_eq!(
        picker.matches.len(),
        CONTENT_ENTRY_LIMIT,
        "the measured final keystroke must still rank the full budget"
    );

    // Warm allocator and worker startup outside the samples. Every sample is
    // an independent copy of the same pre-keystroke state, so all five run
    // the full candidate budget; this is a fixed sample set, not retry-until-
    // success. The median rejects an unrelated scheduler interruption without
    // hiding the complete timing distribution.
    let mut warm = picker.clone();
    warm.insert_query('e');
    let mut samples = Vec::with_capacity(5);
    for _ in 0..5 {
        let mut sample = picker.clone();
        let start = Instant::now();
        sample.insert_query('e');
        samples.push(start.elapsed());
        assert_eq!(sample.matches.len(), CONTENT_ENTRY_LIMIT);
    }
    let mut ordered = samples.clone();
    ordered.sort_unstable();
    let median = ordered[ordered.len() / 2];
    let limit = budget(FRAME * 4);
    eprintln!(
        "a keystroke in content search samples: {samples:?}; median: {median:?}; budget: {limit:?}"
    );
    within("a median keystroke in content search", median, limit);
}

/// Cooperative live-content batches must not re-sort every earlier batch.
/// This fills the same candidate ceiling in the 128-row chunks used by the
/// event loop; a whole-corpus sort after each chunk turns this deliberately
/// broad query into a multi-second foreground stall.
#[test]
#[ignore = "run serially in the release performance job"]
fn incrementally_ranking_a_full_live_content_budget_stays_bounded() {
    const SLICE: usize = 128;
    let mut picker = FilePicker::grep(1, PathBuf::from("/project"));
    picker.insert_query_text("needle");
    picker.finish(0, false);
    // Every batch is built before the clock starts. What is measured is the
    // ranking of each slice and its merge into the already-sorted results;
    // constructing a candidate costs two allocations, and charging 50,000 of
    // those to a ranking budget measures the allocator as much as the merge.
    let batches = || -> Vec<Vec<ResourceItem>> {
        (0..CONTENT_ENTRY_LIMIT)
            .step_by(SLICE)
            .map(|first| {
                let end = (first + SLICE).min(CONTENT_ENTRY_LIMIT);
                (first..end)
                    .map(|row| {
                        ResourceItem::content(
                            format!("scratch:{}", row + 1),
                            format!("needle value {row}"),
                            ResourceTarget::BufferLocation {
                                buffer: 0,
                                row,
                                column: 0,
                            },
                            ResourceKind::Buffer,
                        )
                    })
                    .collect()
            })
            .collect()
    };
    let fill = |finder: &mut ResourceFinder, batches: Vec<Vec<ResourceItem>>| {
        let start = Instant::now();
        for batch in batches {
            finder.append_items(batch, &picker, "needle");
        }
        start.elapsed()
    };

    // Warm the allocator outside the samples, as the whole-corpus ranking
    // budget above does. Every sample fills an empty finder to the same
    // ceiling in the same slices, so this is a fixed sample set rather than
    // retry-until-success; the median rejects a scheduler interruption on a
    // shared runner without hiding the distribution the failure would need.
    let mut warm = ResourceFinder::new(FinderMode::Contents);
    warm.begin_content_scan(&picker, "needle", std::iter::empty());
    fill(&mut warm, batches());
    let mut samples = Vec::with_capacity(5);
    for _ in 0..5 {
        let mut finder = ResourceFinder::new(FinderMode::Contents);
        finder.begin_content_scan(&picker, "needle", std::iter::empty());
        samples.push(fill(&mut finder, batches()));
        assert_eq!(finder.matches.len(), CONTENT_ENTRY_LIMIT);
    }
    let mut ordered = samples.clone();
    ordered.sort_unstable();
    let median = ordered[ordered.len() / 2];
    let limit = budget(FRAME * 30);
    eprintln!(
        "incremental live-content ranking samples: {samples:?}; median: {median:?}; budget: {limit:?}"
    );
    within(
        "incrementally ranking a full live-content budget",
        median,
        limit,
    );
}

// -- Path completion --------------------------------------------------------

/// Entries in the wide fixture directory.
///
/// Far past the few hundred rows either path popup will show, so that the
/// work being measured is the whole directory rather than what survives it.
const WIDE_ENTRIES: usize = 40_000;

/// One directory holding `WIDE_ENTRIES` files and a tenth as many
/// subdirectories, beside the note the editor opens.
fn wide_directory() -> &'static Path {
    static ROOT: OnceLock<PathBuf> = OnceLock::new();
    ROOT.get_or_init(|| {
        let root = fixture_root().join("wide_directory");
        if root.exists() {
            return root;
        }
        let pending = fixture_root().join(format!("wide_directory.{}.pending", std::process::id()));
        let wide = pending.join("wide");
        fs::create_dir_all(&wide).unwrap();
        fs::write(pending.join("note.txt"), "").unwrap();
        for index in 0..WIDE_ENTRIES {
            fs::write(wide.join(format!("file_{index:05}.txt")), "").unwrap();
            if index % 10 == 0 {
                fs::create_dir_all(wide.join(format!("dir_{index:05}"))).unwrap();
            }
        }
        // Renamed into place so a second run never reads a tree this one is
        // still writing, the same way the other fixtures are built.
        if fs::rename(&pending, &root).is_err() {
            fs::remove_dir_all(&pending).ok();
        }
        root
    })
    .as_path()
}

fn editor_in(root: &Path) -> App {
    App::new_in_project(Config::default(), Some(root.join("note.txt")), root).unwrap()
}

fn press(app: &mut App, character: char) {
    app.handle_key(KeyStroke::new(KeyCode::Char(character), Modifiers::NONE))
        .unwrap();
}

fn type_text(app: &mut App, text: &str) {
    for character in text.chars() {
        press(app, character);
    }
}

/// Completing a path in a very wide directory has to stay inside a keystroke.
///
/// Both path popups read the directory whole, because a name being typed can
/// sit anywhere in it and a directory read returns names in no useful order.
/// That read happens on the input thread, between the keystroke and the
/// redraw answering it, so this is what the person waits for. The first read
/// is allowed several frames — it is one directory read of forty thousand
/// entries and nothing can make it free — but every keystroke after it is
/// held to a frame, which is what the kept listing buys.
#[test]
#[ignore = "run serially in the release performance job"]
fn completing_a_path_in_a_wide_directory_stays_within_budget() {
    let root = wide_directory();
    let mut app = editor_in(root);
    press(&mut app, 'i');
    type_text(&mut app, "wide");

    let start = Instant::now();
    press(&mut app, '/');
    within("opening a path popup", start.elapsed(), budget(FRAME * 8));
    assert!(
        app.completion.is_some(),
        "the popup has to open on the separator"
    );

    let slowest = "file_39999"
        .chars()
        .fold(Duration::ZERO, |slowest, character| {
            let start = Instant::now();
            press(&mut app, character);
            slowest.max(start.elapsed())
        });
    within("a keystroke inside a path popup", slowest, budget(FRAME));

    // The whole point of the read is that the name typed is the one offered,
    // however deep in the directory the filesystem happened to put it.
    let state = app.completion.as_ref().expect("the popup stays open");
    let offered = state
        .visible_indices()
        .into_iter()
        .map(|index| state.items[index].label.clone())
        .collect::<Vec<_>>();
    assert_eq!(offered, vec!["file_39999.txt".to_owned()]);
}

/// The palette's rows for a path argument have to stay inside a frame.
///
/// They are recomputed for every frame drawn while the palette is open, so
/// this budget is a redraw budget rather than a keystroke budget, and it
/// covers the widest case: an argument ending in a separator, where every
/// name in the directory is a candidate row.
#[test]
#[ignore = "run serially in the release performance job"]
fn palette_path_rows_redraw_within_budget() {
    let root = wide_directory();
    let mut app = editor_in(root);
    press(&mut app, ':');
    type_text(&mut app, &format!("open {}/wide/", root.display()));

    let first = Instant::now();
    let rows = app
        .matching_path_hints()
        .expect("a path argument owns the rows");
    within("the first path rows", first.elapsed(), budget(FRAME * 8));
    assert_eq!(rows.len(), 512);

    let slowest = (0..8).fold(Duration::ZERO, |slowest, _| {
        let start = Instant::now();
        let rows = app
            .matching_path_hints()
            .expect("a path argument owns the rows");
        assert_eq!(rows.len(), 512);
        slowest.max(start.elapsed())
    });
    within("redrawing path rows", slowest, budget(FRAME * 2));
}
