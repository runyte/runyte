// SPDX-License-Identifier: MPL-2.0

use super::session;
use crate::terminal::TerminalSession;

fn target_at(session: &mut TerminalSession, needle: &str) -> Option<String> {
    session.search_review(needle, false).unwrap();
    let offset = session.review_selection_anchor().unwrap();
    session.goto_review_offset(offset, false);
    session.review_navigation_target()
}

#[test]
fn wrapped_web_links_resolve_from_every_character_in_frozen_history() {
    for url in [
        "https://example.com/a_(b)?q=a,b&x=2#part",
        "http://example.com/long/path",
        "www.example.com/long/path",
        "https://example.com/界/e\u{301}?q=界#part",
    ] {
        for columns in [1, 2, 7, 16, 23] {
            let mut terminal = session(columns, 2);
            terminal.feed(format!("({url}).\r\n").as_bytes());
            terminal.begin_review();
            // Live output and width changes must not alter the frozen target.
            terminal.feed(b"\x1b[2J\x1b[Hchanged");
            terminal.resize(80, 4);
            let review = terminal.review.as_ref().unwrap();
            let offsets: Vec<_> = review
                .text
                .chars()
                .enumerate()
                .filter(|(_, c)| !matches!(c, '\n' | ' '))
                .map(|(offset, _)| offset)
                .collect();
            // Exclude the surrounding opening parenthesis and trailing ').'.
            for &offset in &offsets[1..offsets.len() - 2] {
                terminal.goto_review_offset(offset, false);
                assert_eq!(
                    terminal.review_navigation_target().as_deref(),
                    Some(url),
                    "width {columns}, offset {offset}"
                );
            }
        }
    }
}

#[test]
fn wrapped_links_preserve_spaces_and_real_newlines_as_boundaries() {
    let mut terminal = session(21, 5);
    terminal.feed(b"https://example.com/ \r\nnext\r\n");
    assert_eq!(
        target_at(&mut terminal, "https"),
        Some("https://example.com/".into())
    );
    assert_eq!(target_at(&mut terminal, "next"), Some("next".into()));

    let mut terminal = session(21, 5);
    // The space occupies the final column before an automatic wrap. Review
    // text trims it for display, but inference must preserve that boundary.
    terminal.feed(b"https://example.com/ next");
    assert_eq!(
        target_at(&mut terminal, "https"),
        Some("https://example.com/".into())
    );
    assert_eq!(target_at(&mut terminal, "next"), Some("next".into()));

    let mut terminal = session(21, 5);
    terminal.feed(b"https://example.com/a\r\nnext");
    assert_eq!(
        target_at(&mut terminal, "https"),
        Some("https://example.com/a".into())
    );
    assert_eq!(target_at(&mut terminal, "next"), Some("next".into()));
}

#[test]
fn explicit_wrapped_selections_and_file_tokens_keep_their_existing_text() {
    let mut terminal = session(20, 5);
    terminal.feed(b"https://example.com/long/path");
    terminal.begin_review();
    terminal.set_review_selection(0, 24);
    assert_eq!(
        terminal.review_navigation_target().as_deref(),
        Some("https://example.com/\nlong")
    );

    let mut terminal = session(8, 5);
    terminal.feed(b"some/long/file.txt");
    assert_eq!(target_at(&mut terminal, "some"), Some("some/lon".into()));
}

#[test]
fn wrapped_links_work_on_the_alternate_screen_and_in_insert_mode() {
    for mode in ["\x1b[?1049h", "\x1b[4h"] {
        let mut terminal = session(16, 5);
        terminal.feed(format!("{mode}https://example.com/long/path").as_bytes());
        assert_eq!(
            target_at(&mut terminal, "path"),
            Some("https://example.com/long/path".into())
        );
    }
}

#[test]
fn lost_or_edited_rows_do_not_connect_unrelated_url_fragments() {
    for (edit, expected) in [
        ("\x1b[2;1H\x1b[2K", Some("https://example.com/")), // erase continuation
        ("\x1b[2;1H\x1b[L", Some("https://example.com/")),  // insert intervening row
        ("\x1b[2;1H\x1b[M", Some("https://example.com/")),  // delete continuation
        ("\x1b[2;1H\x1b[2Kreplacement", Some("https://example.com/")), // replace continuation
        ("\x1b[1;1H\x1b[P", Some("ttps://example.com/")),   // shift URL prefix
        ("\x1b[1;1H\x1b[@", None),                          // insert into URL prefix
    ] {
        let mut terminal = session(20, 5);
        terminal.feed(b"https://example.com/long/path/continues");
        terminal.feed(edit.as_bytes());
        terminal.begin_review();
        terminal.goto_review_offset(0, false);
        assert_eq!(
            terminal.review_navigation_target().as_deref(),
            expected,
            "{edit:?}"
        );
    }
    let mut terminal = session(8, 2);
    terminal.feed(b"https://example.com/long/path");
    while terminal.emulator.grid_mut().drop_oldest_scrollback() {}
    assert_eq!(
        target_at(&mut terminal, "long").as_deref(),
        Some("com/long")
    );
}

#[test]
fn width_resize_invalidates_live_wraps_but_height_resize_preserves_them() {
    for columns in [10, 20, 40] {
        let mut terminal = session(20, 5);
        terminal.feed(b"https://example.com/long/path");
        terminal.resize(columns, 2);
        let target = target_at(&mut terminal, "https");
        if columns == 20 {
            assert_eq!(target.as_deref(), Some("https://example.com/long/path"));
        } else {
            assert_ne!(target.as_deref(), Some("https://example.com/long/path"));
        }
    }
}

#[test]
fn line_feed_preserves_existing_wraps() {
    for (setup, movement) in [
        ("", "\x1b[1;20H\n"),
        // A line feed below the scroll region cannot move beyond the screen.
        ("\x1b[3;1H", "\x1b[1;2r\x1b[4;1H\n"),
    ] {
        let mut terminal = session(20, 4);
        terminal.feed(setup.as_bytes());
        terminal.feed(b"https://example.com/long/path");
        terminal.feed(movement.as_bytes());
        assert_eq!(
            target_at(&mut terminal, "https").as_deref(),
            Some("https://example.com/long/path"),
            "{movement:?}"
        );
        assert_eq!(
            target_at(&mut terminal, "path").as_deref(),
            Some("https://example.com/long/path"),
            "{movement:?}"
        );
    }
}

#[test]
fn top_anchored_scroll_regions_keep_wrapped_links_in_history() {
    let mut terminal = session(12, 4);
    terminal.feed(b"\x1b[1;2rhttps://example.com/long/path\r\n");
    assert_eq!(
        target_at(&mut terminal, "https").as_deref(),
        Some("https://example.com/long/path")
    );
}

#[test]
fn partial_erases_preserve_wrapped_urls() {
    for erase in ["\x1b[K", "\x1b[3X", "\x1b[J"] {
        let mut terminal = session(20, 4);
        // A coloured match ends partway through the second row. grep's reset
        // and erase-to-end sequence must not break its link to the first row.
        terminal.feed(b"https://example.com/\x1b[31mlong");
        terminal.feed(format!("\x1b[m{erase}/path").as_bytes());
        assert_eq!(
            target_at(&mut terminal, "path").as_deref(),
            Some("https://example.com/long/path"),
            "{erase:?}"
        );
    }
    // Erasing and redrawing only the beginning of a continuation also keeps
    // the existing relationship to its predecessor.
    let mut terminal = session(20, 4);
    terminal.feed(b"https://example.com/long/path");
    terminal.feed(b"\x1b[2;4H\x1b[1K\x1b[2;1Hlong");
    assert_eq!(
        target_at(&mut terminal, "path").as_deref(),
        Some("https://example.com/long/path")
    );
}

#[test]
fn partial_redraws_preserve_incoming_and_following_wraps() {
    let url = "https://example.com/long/path/continues/aftermore";
    for redraw in ["\x1b[1;1Hhttps", "\x1b[2;1Hlong", "\x1b[1;3Ht"] {
        let mut terminal = session(20, 4);
        terminal.feed(url.as_bytes());
        terminal.feed(redraw.as_bytes());
        assert_eq!(
            target_at(&mut terminal, "https").as_deref(),
            Some(url),
            "{redraw:?}"
        );
        assert_eq!(
            target_at(&mut terminal, "long").as_deref(),
            Some(url),
            "{redraw:?}"
        );
    }
}

#[test]
fn whole_row_erases_break_old_links_even_when_the_row_is_repainted() {
    for erase in [
        "\x1b[2;1H\x1b[2K",
        "\x1b[2;1H\x1b[K",
        "\x1b[2;20H\x1b[1K",
        "\x1b[2;1H\x1b[20X",
        "\x1b[2;1H\x1b[20P",
        "\x1b[2;1H\x1b[20@",
    ] {
        let mut terminal = session(20, 4);
        terminal.feed(b"https://example.com/long/path/continues/aftermore");
        terminal.feed(erase.as_bytes());
        terminal.feed(b"\x1b[2;1Hreplacement");
        assert_eq!(
            target_at(&mut terminal, "https").as_deref(),
            Some("https://example.com/"),
            "{erase:?}"
        );
        assert_eq!(
            target_at(&mut terminal, "replacement").as_deref(),
            Some("replacement")
        );
    }
}

#[test]
fn complete_row_rewrites_break_old_links_independently_of_output_chunks() {
    for chunk_size in [1, 7, 20] {
        let mut terminal = session(20, 4);
        terminal.feed(b"https://example.com/long/path/continues/aftermore");
        terminal.feed(b"\x1b[2;1H");
        for chunk in b"replacement-row-full".chunks(chunk_size) {
            terminal.feed(chunk);
        }
        assert_eq!(
            target_at(&mut terminal, "https").as_deref(),
            Some("https://example.com/")
        );
        assert_eq!(
            target_at(&mut terminal, "replacement").as_deref(),
            Some("replacement-row-full")
        );
    }
}

#[test]
fn partial_character_shifts_preserve_the_remaining_wrapped_url() {
    let mut terminal = session(20, 4);
    terminal.feed(b"https://example.com/long/path");
    terminal.feed(b"\x1b[2;3H\x1b[@\x1b[P");
    assert_eq!(
        target_at(&mut terminal, "path").as_deref(),
        Some("https://example.com/long/path")
    );

    let mut terminal = session(20, 4);
    terminal.feed(b"https://example.com/long/path");
    terminal.feed(b"\x1b[2;3H\x1b[4hn\x1b[4l\x1b[P");
    assert_eq!(
        target_at(&mut terminal, "path").as_deref(),
        Some("https://example.com/long/path")
    );
}

/// Every character of `link` in the review, which starts at its first
/// occurrence and may be interrupted only by line breaks and indentation.
fn link_offsets(terminal: &mut TerminalSession, link: &str) -> Vec<usize> {
    terminal.begin_review();
    let text = terminal.review.as_ref().unwrap().text.clone();
    let start = text[..text.find(&link[..12]).unwrap()].chars().count();
    let offsets = text
        .chars()
        .enumerate()
        .skip(start)
        .filter(|(_, c)| !c.is_whitespace())
        .take(link.chars().count())
        .map(|(offset, _)| offset)
        .collect::<Vec<_>>();
    let found = offsets
        .iter()
        .map(|&offset| text.chars().nth(offset).unwrap())
        .collect::<String>();
    assert_eq!(found, link);
    offsets
}

#[test]
fn links_an_agent_broke_across_indented_rows_resolve_from_every_character() {
    let link = "https://very-long-link-to-some-website-or-file.com";
    let mut terminal = session(60, 8);
    terminal.feed(
        b"    Earlier prose in this block ends at its edge too.\r\n\
          \r\n\
          \x20   This is a section with a https://very-long-link-t\r\n\
          \x20   o-some-website-or-file.com.\r\n",
    );
    for offset in link_offsets(&mut terminal, link) {
        terminal.goto_review_offset(offset, false);
        assert_eq!(
            terminal.review_navigation_target().as_deref(),
            Some(link),
            "offset {offset}"
        );
    }
    assert_eq!(target_at(&mut terminal, "section"), Some("section".into()));

    // A bullet's text continues under its words rather than its marker, and
    // a link may fill whole rows between its first and last.
    let link = "https://example.com/a/very/long/path/that/fills/rows";
    let mut terminal = session(30, 8);
    terminal.feed(
        "⏺ See https://example.com/a/v\r\n  ery/long/path/that/fills/ro\r\n  ws, then prose follows here\r\n"
            .as_bytes(),
    );
    for offset in link_offsets(&mut terminal, link) {
        terminal.goto_review_offset(offset, false);
        assert_eq!(
            terminal.review_navigation_target().as_deref(),
            Some(link),
            "offset {offset}"
        );
    }
    assert_eq!(target_at(&mut terminal, "prose"), Some("prose".into()));
}

#[test]
fn an_indented_link_ending_before_the_wrap_edge_is_not_joined() {
    let mut terminal = session(60, 8);
    // The link's row ends short of the edge the block wraps at, so the next
    // word moved down for want of room: the break is an ordinary space.
    terminal.feed(
        b"    A longer row shows where this block wraps its lines at.\r\n\
          \x20   see https://example.com/page\r\n\
          \x20   for details.\r\n",
    );
    assert_eq!(
        target_at(&mut terminal, "https"),
        Some("https://example.com/page".into())
    );
    assert_eq!(target_at(&mut terminal, "for"), Some("for".into()));

    // An unindented row is never continued, even when it is full.
    let mut terminal = session(24, 4);
    terminal.feed(b"https://example.com/abcd\r\nnext\r\n");
    assert_eq!(
        target_at(&mut terminal, "https"),
        Some("https://example.com/abcd".into())
    );

    // Nor is a row indented differently from the text above it.
    let mut terminal = session(30, 4);
    terminal.feed(b"  go https://example.com/abcde\r\n    next\r\n");
    assert_eq!(
        target_at(&mut terminal, "https"),
        Some("https://example.com/abcde".into())
    );
}
