// SPDX-License-Identifier: MPL-2.0

use super::*;

/// Two files, a wide enough screen, and a frame prepared so panes have
/// geometry: the state every comparison test starts from.
fn compared(left: &str, right: &str) -> (App, PathBuf, PathBuf) {
    let directory = temporary("diff-two-files");
    fs::create_dir_all(&directory).unwrap();
    let one = directory.join("one.txt");
    let two = directory.join("two.txt");
    fs::write(&one, left).unwrap();
    fs::write(&two, right).unwrap();
    let mut app = App::new(Config::default(), Some(one.clone())).unwrap();
    prepare(&mut app);
    app.execute_command("diff-this").unwrap();
    app.open_file(two.clone()).unwrap();
    app.execute_command("diff-this").unwrap();
    prepare(&mut app);
    (app, one, two)
}

fn prepare(app: &mut App) -> PreparedView {
    app.prepare_view(FrameGeometry {
        screen: Rect {
            width: 80,
            height: 22,
            ..Rect::default()
        },
        editor: Rect {
            width: 80,
            height: 20,
            ..Rect::default()
        },
        status: Rect::default(),
        message: Rect::default(),
    })
}

/// The rows each pane shows, by document row, with `None` for filler.
fn rows(view: &PreparedView, pane: usize, count: usize) -> Vec<Option<usize>> {
    view.pane(pane)
        .unwrap()
        .rows
        .iter()
        .take(count)
        .map(|row| row.document_row)
        .collect()
}

fn sides(app: &App) -> (usize, usize) {
    let session = app.diffs.first().expect("a comparison is open");
    (
        session.side(Side::Left).pane,
        session.side(Side::Right).pane,
    )
}

/// The second `:diff-this` is what opens the view, and it splits by itself
/// when the buffer marked first is not on screen.
#[test]
fn marking_two_buffers_opens_them_side_by_side() {
    let (app, _, _) = compared("a\nb\nc\n", "a\nb\nc\n");
    assert_eq!(app.diffs.len(), 1);
    assert_eq!(app.panes.len(), 2);
    let (left, right) = sides(&app);
    assert_ne!(left, right);
    // Left and right are read off the screen, so the left side really is
    // the pane the person sees on the left.
    assert!(app.areas[&left].x < app.areas[&right].x);
    assert!(app.status.contains("identical"), "{}", app.status);
}

/// Equal lines sit level, and a line only one side has holds the other
/// side open rather than pushing everything below it out of step.
#[test]
fn a_line_only_one_side_has_holds_the_other_side_open() {
    let (mut app, _, _) = compared("a\nc\n", "a\nb\nc\n");
    let view = prepare(&mut app);
    let (left, right) = sides(&app);
    assert_eq!(rows(&view, left, 3), [Some(0), None, Some(1)]);
    assert_eq!(rows(&view, right, 3), [Some(0), Some(1), Some(2)]);
}

/// The colours each side carries are about that side: the right one gained
/// a line, the left one is missing it.
#[test]
fn each_side_is_coloured_by_what_it_has() {
    let (mut app, _, _) = compared("a\nc\n", "a\nb\nc\n");
    let view = prepare(&mut app);
    let snapshot = app.snapshot(&view);
    let (left, right) = sides(&app);
    let compared_at =
        |pane: usize, screen_row: usize| match &snapshot.pane(pane).unwrap().rows[screen_row] {
            crate::snapshot::SnapshotRow::Text(row) => row.compared,
            crate::snapshot::SnapshotRow::Filler
            | crate::snapshot::SnapshotRow::Padding
            | crate::snapshot::SnapshotRow::Placeholder => None,
        };
    assert!(matches!(
        snapshot.pane(left).unwrap().rows[1],
        crate::snapshot::SnapshotRow::Filler
    ));
    assert_eq!(compared_at(right, 1), Some(crate::diff::Change::Added));
    assert_eq!(compared_at(right, 0), None);
    assert_eq!(compared_at(left, 0), None);
}

/// Scrolling one side moves the other to the line facing it, which is not
/// the same row number once the two files have drifted apart.
#[test]
fn scrolling_one_side_moves_the_other_to_the_line_facing_it() {
    let left_text = (0..40).map(|line| format!("{line}\n")).collect::<String>();
    // Three lines the right side has and the left one does not, up at the
    // top, so every row below them faces a row three further down.
    let right_text = format!("x\ny\nz\n{left_text}");
    let (mut app, _, _) = compared(&left_text, &right_text);
    let (left, right) = sides(&app);

    app.activate_pane(left);
    app.panes.get_mut(&left).unwrap().scroll_row = 10;
    app.panes.get_mut(&left).unwrap().preserve_scroll = true;
    prepare(&mut app);
    assert_eq!(app.panes[&right].scroll_row, 13);

    // And the other way round: whichever pane is active leads.
    app.activate_pane(right);
    app.panes.get_mut(&right).unwrap().scroll_row = 20;
    app.panes.get_mut(&right).unwrap().preserve_scroll = true;
    prepare(&mut app);
    assert_eq!(app.panes[&left].scroll_row, 17);
}

/// Both sides stay editable, and the alignment follows the text rather
/// than describing what the files used to say.
#[test]
fn editing_a_side_realigns_the_view() {
    let (mut app, _, _) = compared("a\nb\n", "a\nb\n");
    assert!(app.diffs[0].alignment().is_equal());
    app.mode = Mode::Insert;
    app.insert_text("new\n");
    prepare(&mut app);
    assert!(!app.diffs[0].alignment().is_equal());
    let session = &app.diffs[0];
    let side = session.side_of_pane(app.active_pane).unwrap();
    assert_eq!(session.change(side, 0), Some(crate::diff::Change::Added));
}

/// Alignment is line-based, so a diff pane does not wrap however the
/// editor is configured — and it wraps again once the view closes.
#[test]
fn a_diff_pane_does_not_soft_wrap() {
    let (mut app, _, _) = compared("a\n", "a\n");
    app.config.editor.soft_wrap = true;
    let (left, right) = sides(&app);
    assert!(!app.pane_soft_wrap(left));
    assert!(!app.pane_soft_wrap(right));
    app.execute_command("diff-off").unwrap();
    assert!(app.pane_soft_wrap(left));
}

#[test]
fn marking_the_same_buffer_again_takes_the_mark_back() {
    let directory = temporary("diff-unmark");
    fs::create_dir_all(&directory).unwrap();
    let path = directory.join("one.txt");
    fs::write(&path, "a\n").unwrap();
    let mut app = App::new(Config::default(), Some(path)).unwrap();
    app.execute_command("diff-this").unwrap();
    assert!(app.pending_diff.is_some());
    app.execute_command("diff-this").unwrap();
    assert!(app.pending_diff.is_none());
    assert!(app.diffs.is_empty());
    assert!(app.status.contains("no longer marked"), "{}", app.status);
    fs::remove_dir_all(directory).unwrap();
}

/// `:diff-off` closes the view, and the panes stay where they are: closing
/// a comparison is not closing a window.
#[test]
fn diff_off_closes_the_comparison_and_keeps_both_panes() {
    let (mut app, _, _) = compared("a\n", "b\n");
    app.execute_command("diff-off").unwrap();
    assert!(app.diffs.is_empty());
    assert_eq!(app.panes.len(), 2);
    app.execute_command("diff-off").unwrap();
    assert!(app.status.contains("no comparison"), "{}", app.status);
}

/// A comparison needs both its panes. Closing one is not a diff command,
/// so the view has to notice on its own rather than leaving a session
/// pointing at a pane that is gone.
#[test]
fn closing_a_pane_ends_the_comparison() {
    let (mut app, _, _) = compared("a\n", "b\n");
    app.close_pane();
    prepare(&mut app);
    assert!(app.diffs.is_empty());
}

/// Retargeting a pane at another buffer ends the comparison too: the view
/// was about those two buffers, and one of them is no longer on screen.
#[test]
fn showing_another_buffer_ends_the_comparison() {
    let (mut app, one, _) = compared("a\n", "b\n");
    let (left, _) = sides(&app);
    app.activate_pane(left);
    app.buffers.push(Buffer::scratch());
    app.syntax.push(None);
    let scratch = app.buffers.len() - 1;
    app.panes.get_mut(&left).unwrap().retarget(scratch);
    prepare(&mut app);
    assert!(app.diffs.is_empty());
    drop(one);
}

/// Two buffers already in two panes keep the panes they are in, and the
/// one on the left is the one on the left however they were marked.
#[test]
fn buffers_already_in_two_panes_keep_them() {
    let directory = temporary("diff-existing-panes");
    fs::create_dir_all(&directory).unwrap();
    let one = directory.join("one.txt");
    let two = directory.join("two.txt");
    fs::write(&one, "a\nc\n").unwrap();
    fs::write(&two, "a\nb\nc\n").unwrap();
    let mut app = App::new(Config::default(), Some(one)).unwrap();
    let first = app.active_pane;
    app.split(Axis::Horizontal, None).unwrap();
    let second = app.active_pane;
    app.open_file(two).unwrap();
    prepare(&mut app);

    // Marked in the right-hand pane first, so the mark order and the
    // screen order disagree; the screen order is what decides.
    app.execute_command("diff-this").unwrap();
    app.activate_pane(first);
    app.execute_command("diff-this").unwrap();

    assert_eq!(app.panes.len(), 2, "an existing pane was not reused");
    let (left, right) = sides(&app);
    assert_eq!((left, right), (first, second));
    let view = prepare(&mut app);
    assert_eq!(rows(&view, first, 3), [Some(0), None, Some(1)]);
    assert_eq!(rows(&view, second, 3), [Some(0), Some(1), Some(2)]);
    fs::remove_dir_all(directory).unwrap();
}

/// The two spellings the report asked for reach the same command.
#[test]
fn the_comparison_commands_answer_to_their_short_spellings() {
    for (spelling, id) in [
        ("diff-this", ColonCommand::DiffThis),
        ("difft", ColonCommand::DiffThis),
        ("dt", ColonCommand::DiffThis),
        ("diff-off", ColonCommand::DiffOff),
        ("do", ColonCommand::DiffOff),
    ] {
        assert_eq!(
            parse_colon_command(spelling).unwrap().id(),
            CommandId::Colon(id),
            "{spelling}"
        );
    }
}

/// A directory listing has no lines to compare, and a buffer already in a
/// comparison is not marked for a second one.
#[test]
fn what_cannot_be_compared_says_so() {
    let (mut app, _, _) = compared("a\n", "b\n");
    app.execute_command("diff-this").unwrap();
    assert!(
        app.status.contains("already being compared"),
        "{}",
        app.status
    );
    assert_eq!(app.diffs.len(), 1);
}
