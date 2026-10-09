// SPDX-License-Identifier: MPL-2.0

//! Text objects read from the characters alone: words, paragraphs, and
//! balanced delimiter pairs.
//!
//! These are what `m i` and `m a` select when no syntax tree can answer. Words
//! and paragraphs never need one, so they are always read here. A delimiter
//! pair prefers the syntax tree, which knows a bracket inside a string from
//! one in code, and falls back to the balanced scan below when a buffer has no
//! tree or the tree finds no enclosing pair.
//!
//! Knows nothing about syntax trees, buffers, panes, or selections. Every span
//! is half-open, in character offsets.

use crate::text::{Offset, Text};

/// How far a balanced-bracket scan reaches on each side of the selection.
///
/// A scan reads every character it covers, so an unbounded one would make a
/// keystroke in a minified or very large file cost the whole file. A pair
/// whose delimiters are further apart than this is not found.
pub const PAIR_REACH: usize = 1 << 16;

/// Whether an object includes its surroundings.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Part {
    Inside,
    Around,
}

/// A half-open span of character offsets.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Span {
    pub from: Offset,
    pub to: Offset,
}

impl Span {
    pub const fn new(from: Offset, to: Offset) -> Self {
        Self { from, to }
    }

    pub const fn len(self) -> usize {
        self.to.saturating_sub(self.from)
    }

    pub const fn is_empty(self) -> bool {
        self.to <= self.from
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum WordClass {
    LineEnding,
    Whitespace,
    Word,
    Punctuation,
}

fn word_class(character: char, long: bool) -> WordClass {
    if matches!(character, '\n' | '\r') {
        WordClass::LineEnding
    } else if character.is_whitespace() {
        WordClass::Whitespace
    } else if long || character.is_alphanumeric() || character == '_' {
        WordClass::Word
    } else {
        WordClass::Punctuation
    }
}

/// The run of characters sharing `offset`'s class, never crossing a line.
fn run(text: &Text, offset: Offset, long: bool) -> Span {
    let class = class_at(text, offset, long);
    let same = |at: Offset| class_at(text, at, long) == class;
    let mut from = offset;
    while from > 0 && same(from - 1) {
        from -= 1;
    }
    let mut to = offset + 1;
    while to < text.len_chars() && same(to) {
        to += 1;
    }
    Span::new(from, to)
}

/// The class of the user-perceived character containing `offset`. An accent
/// or the parts of a joined emoji take the class of the character they
/// continue, so a run never ends inside one.
fn class_at(text: &Text, offset: Offset, long: bool) -> Option<WordClass> {
    text.char_at(text.grapheme_floor(offset))
        .map(|ch| word_class(ch, long))
}

/// The word under `offset`, as Vim's `iw` and `aw` read it.
///
/// Inside is the run of characters of one class: a word, a run of punctuation,
/// or a run of whitespace. With `long`, punctuation joins words, so the run is
/// everything between whitespace. Around adds the whitespace after a word, or
/// the whitespace before it when nothing follows on the line; around a run of
/// whitespace adds the word after it, or the one before it at a line's end.
/// Nothing crosses a line break, and a caret on one has no word.
pub fn word(text: &Text, offset: Offset, long: bool, part: Part) -> Option<Span> {
    let class = class_at(text, offset, long)?;
    if class == WordClass::LineEnding {
        return None;
    }
    let inner = run(text, offset, long);
    if part == Part::Inside {
        return Some(inner);
    }
    let after = class_at(text, inner.to, long);
    let before = inner
        .from
        .checked_sub(1)
        .and_then(|at| class_at(text, at, long));
    let joins = |neighbour: Option<WordClass>| match neighbour {
        None | Some(WordClass::LineEnding) => false,
        Some(WordClass::Whitespace) => class != WordClass::Whitespace,
        Some(_) => class == WordClass::Whitespace,
    };
    Some(if joins(after) {
        Span::new(inner.from, run(text, inner.to, long).to)
    } else if joins(before) {
        Span::new(run(text, inner.from - 1, long).from, inner.to)
    } else {
        inner
    })
}

/// The last row that holds text of its own. The empty row after a final line
/// terminator is where the caret goes to append, not a line of the document.
fn last_text_row(text: &Text) -> usize {
    let last = text.last_row();
    if last > 0 && text.line_len(last) == 0 {
        last - 1
    } else {
        last
    }
}

fn is_blank_row(text: &Text, row: usize) -> bool {
    text.line(row).chars().all(char::is_whitespace)
}

/// The rows of the paragraph at `row`, first and last inclusive.
///
/// A paragraph is a run of rows holding text, separated by rows that are empty
/// or hold only whitespace. Inside is the run itself; on a blank row it is the
/// run of blank rows. Around adds the blank rows after a paragraph, or before
/// it when it ends the document; around a blank run adds the paragraph after
/// it, or the one before it at the end. `None` only for an empty document.
pub fn paragraph_rows(text: &Text, row: usize, part: Part) -> Option<(usize, usize)> {
    if text.is_empty() {
        return None;
    }
    let last_row = last_text_row(text);
    let row = row.min(last_row);
    let blank = is_blank_row(text, row);
    let extent = |row: usize, blank: bool| {
        let mut first = row;
        while first > 0 && is_blank_row(text, first - 1) == blank {
            first -= 1;
        }
        let mut last = row;
        while last < last_row && is_blank_row(text, last + 1) == blank {
            last += 1;
        }
        (first, last)
    };
    let (first, last) = extent(row, blank);
    if part == Part::Inside {
        return Some((first, last));
    }
    Some(if last < last_row {
        (first, extent(last + 1, !blank).1)
    } else if first > 0 {
        (extent(first - 1, !blank).0, last)
    } else {
        (first, last)
    })
}

/// Where a scan for a pair of `open` and `close` around `range` looks.
///
/// Quotes do not nest, so which quote closes which is only known by counting
/// from a fixed start; the start of the line is the one a reader counts from,
/// and a quoted string rarely spans lines. Brackets nest and may span many
/// lines, so they are scanned within `PAIR_REACH` on either side.
pub fn pair_bounds(text: &Text, range: Span, open: char, close: char) -> Span {
    if open == close {
        let first = text.offset_to_row(range.from);
        let last = text.offset_to_row(range.to);
        Span::new(
            text.line_to_offset(first),
            text.line_to_offset(last) + text.line_len(last),
        )
    } else {
        Span::new(
            range.from.saturating_sub(PAIR_REACH),
            range.to.saturating_add(PAIR_REACH).min(text.len_chars()),
        )
    }
}

/// The smallest pair of `open` and `close` within `bounds` that encloses
/// `range` and differs from it, so asking again grows to the next pair out.
///
/// An empty `range` is a caret, enclosed when the pair covers the character
/// under it, delimiters included. A delimiter escaped by an odd run of
/// backslashes is text rather than a delimiter. When `open` and `close` are
/// the same character, successive occurrences pair up from the start of
/// `bounds`.
pub fn enclosing_pair(
    text: &Text,
    bounds: Span,
    range: Span,
    (open, close): (char, char),
    part: Part,
) -> Option<Span> {
    let bounds = Span::new(bounds.from, bounds.to.min(text.len_chars()));
    let encloses = |around: Span| {
        if range.is_empty() {
            around.from <= range.from && range.from < around.to
        } else {
            around.from <= range.from && range.to <= around.to
        }
    };
    let mut best: Option<Span> = None;
    let mut consider = |around: Span| {
        if !encloses(around) {
            return;
        }
        let selected = match part {
            Part::Around => around,
            Part::Inside => Span::new(around.from + 1, around.to - 1),
        };
        if selected != range && best.is_none_or(|best| selected.len() < best.len()) {
            best = Some(selected);
        }
    };

    let mut openings: Vec<Offset> = Vec::new();
    let mut backslashes = 0usize;
    let characters = text
        .rope()
        .chars_at(bounds.from)
        .take(bounds.len())
        .enumerate();
    for (relative, character) in characters {
        let offset = bounds.from + relative;
        let escaped = backslashes % 2 == 1;
        backslashes = if character == '\\' {
            backslashes + 1
        } else {
            0
        };
        if escaped {
            continue;
        }
        if open == close {
            if character != open {
                continue;
            }
            match openings.pop() {
                Some(from) => consider(Span::new(from, offset + 1)),
                None => openings.push(offset),
            }
        } else if character == open {
            openings.push(offset);
        } else if character == close
            && let Some(from) = openings.pop()
        {
            consider(Span::new(from, offset + 1));
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(source: &str) -> Text {
        Text::from_str(source)
    }

    fn slice(text: &Text, span: Span) -> String {
        text.slice_string(span.from, span.to)
    }

    fn word_at(source: &str, offset: Offset, long: bool, part: Part) -> Option<String> {
        let text = text(source);
        word(&text, offset, long, part).map(|span| slice(&text, span))
    }

    #[test]
    fn inside_word_is_the_run_of_one_class_under_the_caret() {
        assert_eq!(
            word_at("let snake_case = 1;", 6, false, Part::Inside).as_deref(),
            Some("snake_case")
        );
        assert_eq!(
            word_at("a.b(c)", 1, false, Part::Inside).as_deref(),
            Some(".")
        );
        assert_eq!(
            word_at("a.b(c) d", 1, true, Part::Inside).as_deref(),
            Some("a.b(c)")
        );
        assert_eq!(
            word_at("one   two", 4, false, Part::Inside).as_deref(),
            Some("   ")
        );
        assert_eq!(
            word_at("zażółć gęślą", 2, false, Part::Inside).as_deref(),
            Some("zażółć")
        );
    }

    #[test]
    fn around_word_takes_the_space_after_or_else_before() {
        assert_eq!(
            word_at("one two three", 5, false, Part::Around).as_deref(),
            Some("two ")
        );
        assert_eq!(
            word_at("one two", 5, false, Part::Around).as_deref(),
            Some(" two")
        );
        // Punctuation after the word is not space to take, so the space
        // before is taken instead.
        assert_eq!(
            word_at("call foo(x)", 6, false, Part::Around).as_deref(),
            Some(" foo")
        );
        // Around a run of whitespace, the word after it joins.
        assert_eq!(
            word_at("one   two", 4, false, Part::Around).as_deref(),
            Some("   two")
        );
        assert_eq!(
            word_at("one   \nnext", 4, false, Part::Around).as_deref(),
            Some("one   ")
        );
        assert_eq!(
            word_at("alone\nnext", 1, false, Part::Around).as_deref(),
            Some("alone")
        );
    }

    #[test]
    fn a_word_never_crosses_a_line_break() {
        assert_eq!(word_at("one\ntwo", 3, false, Part::Inside), None);
        assert_eq!(word_at("one\r\ntwo", 3, false, Part::Around), None);
        assert_eq!(
            word_at("one\ntwo", 2, false, Part::Around).as_deref(),
            Some("one")
        );
        assert_eq!(word_at("", 0, false, Part::Inside), None);
    }

    #[test]
    fn paragraph_is_a_run_of_rows_holding_text() {
        let source = "one\ntwo\n\n  \nthree\nfour\n";
        let text = text(source);
        assert_eq!(paragraph_rows(&text, 1, Part::Inside), Some((0, 1)));
        assert_eq!(paragraph_rows(&text, 0, Part::Around), Some((0, 3)));
        // A whitespace-only row is blank.
        assert_eq!(paragraph_rows(&text, 2, Part::Inside), Some((2, 3)));
        assert_eq!(paragraph_rows(&text, 3, Part::Around), Some((2, 5)));
        // The last paragraph has no blank rows after it, so around takes the
        // ones before it; the empty row after the final terminator is not one.
        assert_eq!(paragraph_rows(&text, 5, Part::Inside), Some((4, 5)));
        assert_eq!(paragraph_rows(&text, 6, Part::Inside), Some((4, 5)));
        assert_eq!(paragraph_rows(&text, 4, Part::Around), Some((2, 5)));
    }

    #[test]
    fn paragraph_edges_and_an_empty_document() {
        assert_eq!(paragraph_rows(&text(""), 0, Part::Inside), None);
        assert_eq!(paragraph_rows(&text("only"), 0, Part::Around), Some((0, 0)));
        assert_eq!(
            paragraph_rows(&text("\n\nbody"), 0, Part::Around),
            Some((0, 2))
        );
    }

    fn pair(source: &str, range: Span, delimiters: (char, char), part: Part) -> Option<String> {
        let text = text(source);
        let bounds = pair_bounds(&text, range, delimiters.0, delimiters.1);
        enclosing_pair(&text, bounds, range, delimiters, part).map(|span| slice(&text, span))
    }

    #[test]
    fn enclosing_pair_finds_the_smallest_balanced_pair() {
        let source = "f(a, (b + c), d)";
        let caret = Span::new(7, 7);
        assert_eq!(
            pair(source, caret, ('(', ')'), Part::Inside).as_deref(),
            Some("b + c")
        );
        assert_eq!(
            pair(source, caret, ('(', ')'), Part::Around).as_deref(),
            Some("(b + c)")
        );
        // On a delimiter, its own pair encloses the caret.
        assert_eq!(
            pair(source, Span::new(5, 5), ('(', ')'), Part::Around).as_deref(),
            Some("(b + c)")
        );
        assert_eq!(
            pair(source, Span::new(0, 0), ('(', ')'), Part::Inside),
            None
        );
    }

    #[test]
    fn asking_again_grows_to_the_next_pair_out() {
        let source = "f(a, (b + c), d)";
        assert_eq!(
            pair(source, Span::new(6, 11), ('(', ')'), Part::Inside).as_deref(),
            Some("a, (b + c), d")
        );
        assert_eq!(
            pair(source, Span::new(5, 12), ('(', ')'), Part::Around).as_deref(),
            Some("(a, (b + c), d)")
        );
    }

    #[test]
    fn brackets_span_lines_and_quotes_pair_within_one() {
        let source = "{\n  one,\n  two\n}";
        assert_eq!(
            pair(source, Span::new(4, 4), ('{', '}'), Part::Inside).as_deref(),
            Some("\n  one,\n  two\n")
        );
        let quoted = "say \"hi\" and \"bye\"\nthen \"more\"";
        assert_eq!(
            pair(quoted, Span::new(6, 6), ('"', '"'), Part::Inside).as_deref(),
            Some("hi")
        );
        assert_eq!(
            pair(quoted, Span::new(15, 15), ('"', '"'), Part::Around).as_deref(),
            Some("\"bye\"")
        );
        // Between two strings the caret is in neither.
        assert_eq!(
            pair(quoted, Span::new(10, 10), ('"', '"'), Part::Inside),
            None
        );
        // A quote on another line does not pair with one on this line.
        assert_eq!(
            pair(quoted, Span::new(26, 26), ('"', '"'), Part::Inside).as_deref(),
            Some("more")
        );
    }

    #[test]
    fn escaped_delimiters_are_text() {
        let source = r#"x = "a \" b" + (c \) d)"#;
        assert_eq!(
            pair(source, Span::new(6, 6), ('"', '"'), Part::Inside).as_deref(),
            Some(r#"a \" b"#)
        );
        assert_eq!(
            pair(source, Span::new(16, 16), ('(', ')'), Part::Inside).as_deref(),
            Some(r"c \) d")
        );
    }

    #[test]
    fn a_pair_beyond_the_reach_is_not_found() {
        let mut source = String::from("(");
        source.push_str(&"x".repeat(PAIR_REACH + 1));
        source.push(')');
        assert_eq!(
            pair(&source, Span::new(1, 1), ('(', ')'), Part::Inside),
            None
        );
    }
}
