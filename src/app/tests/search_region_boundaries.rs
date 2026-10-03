// SPDX-License-Identifier: MPL-2.0

use super::*;
use crate::test_support::TestRuntimeRoot;

#[test]
fn expanded_syntax_search_excludes_the_half_open_boundary_in_preview_and_repeats() {
    let root = TestRuntimeRoot::new("search-syntax").unwrap();
    let path = root.path().join("note.rs");
    fs::write(&path, "fn demo() {}\n").unwrap();
    let mut app = App::new(Config::default(), Some(path)).unwrap();
    app.active_mut().replace_selection(Selection::point(3));
    app.expand_syntax_selection().unwrap();
    assert_eq!(
        app.active().selection_semantics(),
        SelectionSemantics::HalfOpen
    );
    assert_eq!(app.operative_spans(), vec![(3, 7)]);

    press(&mut app, 's');
    app.command = "(".into();
    app.refresh_search_preview();
    assert_eq!(
        app.search_preview.as_ref().unwrap().role_at(7),
        crate::snapshot::TextRole::Plain
    );

    app.command = "demo".into();
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert_eq!(app.search_matches().unwrap(), vec![Range::new(3, 6)]);
    app.edit(Transaction::insert(0, "// α\n"));
    assert_eq!(app.search_matches().unwrap(), vec![Range::new(8, 11)]);
    app.search.pattern = "(".into();
    assert!(app.search_matches().unwrap().is_empty());
}

#[test]
fn a_one_character_half_open_selection_does_not_scope_the_search() {
    let mut app = App::new(Config::default(), None).unwrap();
    seed(&mut app, "a b");
    app.active_mut()
        .replace_selection(Selection::single(Range::new(0, 1)));
    app.active_mut()
        .mark_selection_semantics(SelectionSemantics::HalfOpen);
    press(&mut app, 's');
    app.command = "b".into();
    key(&mut app, KeyCode::Enter, Modifiers::NONE);
    assert_eq!(app.active().selection.primary(), Range::point(2));
    assert!(app.search.region.is_none());
}
