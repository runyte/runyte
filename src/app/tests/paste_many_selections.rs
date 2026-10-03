// SPDX-License-Identifier: MPL-2.0

use super::*;

#[test]
fn pasting_many_search_matches_selects_each_unicode_replacement_and_undoes_once() {
    let count = 20_000;
    let original = "a ".repeat(count);
    let mut app = App::new(Config::default(), None).unwrap();
    seed(&mut app, &original);
    app.buffers[0].commit_undo_group();
    app.active_mut().replace_selection(Selection::new(
        (0..count).map(|i| Range::point(i * 2)).collect(),
        count / 2,
    ));
    app.mode = Mode::Select;
    let register = Register {
        text: "λλ".into(),
        ..Register::default()
    };
    app.paste_register(&register, false, false);
    assert_eq!(text(&app), "λλ ".repeat(count));
    assert_eq!(app.active().selection.primary_index(), count / 2);
    assert_eq!(app.active().selection.len(), count);
    for (i, range) in app.active().selection.ranges().iter().enumerate() {
        assert_eq!(*range, Range::new(i * 3, i * 3 + 1));
    }
    app.undo();
    assert_eq!(text(&app), original);
}
