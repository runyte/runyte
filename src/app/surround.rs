// SPDX-License-Identifier: MPL-2.0

//! Surround editing: adding, replacing, and deleting the pair of delimiters
//! around each selection, as Helix's `ms`, `mr`, and `md` do.
//!
//! A pair to replace or delete is found the way `m a` finds one, through the
//! syntax tree first and then by a balanced scan of the text, so the two can
//! never disagree about which pair encloses a cursor.

use super::{
    App, Change, DelimiterPair, Mode, Range, Result, Selection, SelectionSemantics,
    SyntaxObjectPart, Transaction, enclosing_delimiter,
};
use crate::syntax::SyntaxRange;

/// The delimiters `ms` and the second key of `mr` insert for `character`.
/// Either bracket of a pair names the pair; any other character is used on
/// both sides, so `ms*` emphasises and `ms|` fences.
fn inserted_pair(character: char) -> (char, char) {
    match character {
        '(' | ')' => ('(', ')'),
        '[' | ']' => ('[', ']'),
        '{' | '}' => ('{', '}'),
        '<' | '>' => ('<', '>'),
        other => (other, other),
    }
}

/// The pair `md` and the first key of `mr` look for: one of the seven
/// `m i`/`m a` understands, or `None` for `m`, the closest of any of them.
fn existing_pair(character: char) -> Option<Option<DelimiterPair>> {
    Some(match character {
        '(' | ')' => Some(DelimiterPair::Parentheses),
        '[' | ']' => Some(DelimiterPair::SquareBrackets),
        '{' | '}' => Some(DelimiterPair::Braces),
        '<' | '>' => Some(DelimiterPair::AngleBrackets),
        '"' => Some(DelimiterPair::DoubleQuotes),
        '\'' => Some(DelimiterPair::SingleQuotes),
        '`' => Some(DelimiterPair::Backticks),
        'm' => None,
        _ => return None,
    })
}

fn pair_label(pair: Option<DelimiterPair>) -> &'static str {
    match pair {
        Some(DelimiterPair::Parentheses) => "parentheses",
        Some(DelimiterPair::SquareBrackets) => "square brackets",
        Some(DelimiterPair::Braces) => "braces",
        Some(DelimiterPair::AngleBrackets) => "angle brackets",
        Some(DelimiterPair::DoubleQuotes) => "double quotes",
        Some(DelimiterPair::SingleQuotes) => "single quotes",
        Some(DelimiterPair::Backticks) => "backticks",
        None => "pair",
    }
}

impl App {
    /// Wraps each selection in the pair `character` names and selects the
    /// result, delimiters included, so `md` or `mr` can act on it next.
    pub(super) fn surround_add(&mut self, character: char) {
        if self.refuse_read_only_surround() {
            return;
        }
        let (open, close) = inserted_pair(character);
        let selection = self.active().selection.clone();
        // Selections never overlap, but an inclusive span can reach a caret
        // sitting on its last character. Such spans are wrapped together.
        let mut spans: Vec<(usize, usize, bool)> = Vec::new();
        let mut directed: Vec<_> = self
            .operative_spans()
            .into_iter()
            .zip(selection.ranges())
            .map(|((from, to), range)| (from, to, range.anchor > range.head))
            .collect();
        directed.sort_by_key(|(from, to, _)| (*from, *to));
        for (from, to, reversed) in directed {
            match spans.last_mut() {
                Some(previous) if from < previous.1 => previous.1 = previous.1.max(to),
                _ => spans.push((from, to, reversed)),
            }
        }

        let mut changes = Vec::with_capacity(spans.len() * 2);
        let mut ranges = Vec::with_capacity(spans.len());
        let mut shift = 0;
        for (from, to, reversed) in spans {
            changes.push(Change::new(from, from, open.to_string()));
            changes.push(Change::new(to, to, close.to_string()));
            // Each earlier span has added two characters before this one.
            let first = from + shift;
            let last = to + shift + 1;
            shift += 2;
            ranges.push(if reversed {
                Range::new(last, first)
            } else {
                Range::new(first, last)
            });
        }
        if !self.edit(Transaction::new(changes)) {
            return;
        }
        let buffer_id = self.active().buffer;
        self.active_mut()
            .replace_selection(Selection::new(ranges, selection.primary_index()));
        self.active_mut()
            .mark_selection_semantics(SelectionSemantics::Runyte);
        self.mode = Mode::Normal;
        self.normalize_buffer(buffer_id);
    }

    /// Replaces the delimiters of the pair `from` names around each cursor
    /// with the pair `to` names, leaving what they enclose alone.
    pub(super) fn surround_replace(&mut self, from: char, to: char) -> Result<()> {
        let Some(spans) = self.surrounding_pairs(from)? else {
            return Ok(());
        };
        let (open, close) = inserted_pair(to);
        let changes = spans
            .into_iter()
            .flat_map(|span| {
                [
                    Change::new(span.from, span.from + 1, open.to_string()),
                    Change::new(span.to - 1, span.to, close.to_string()),
                ]
            })
            .collect();
        self.finish_surround_edit(changes);
        Ok(())
    }

    /// Deletes the delimiters of the pair `character` names around each
    /// cursor, leaving what they enclose.
    pub(super) fn surround_delete(&mut self, character: char) -> Result<()> {
        let Some(spans) = self.surrounding_pairs(character)? else {
            return Ok(());
        };
        let changes = spans
            .into_iter()
            .flat_map(|span| {
                [
                    Change::new(span.from, span.from + 1, ""),
                    Change::new(span.to - 1, span.to, ""),
                ]
            })
            .collect();
        self.finish_surround_edit(changes);
        Ok(())
    }

    fn finish_surround_edit(&mut self, changes: Vec<Change>) {
        // Cursors inside one pair find it once each; the transaction keeps the
        // first of each identical change, so the pair is edited once.
        let buffer_id = self.active().buffer;
        self.edit(Transaction::new(changes));
        self.mode = Mode::Normal;
        self.normalize_buffer(buffer_id);
    }

    /// The pair around the character under each cursor, delimiters
    /// included, or `None` after reporting why nothing will change.
    ///
    /// The lookup starts from that character rather than from the whole
    /// selection, so a pair `m a` has just selected is the pair found rather
    /// than the next one out. Every cursor has to find a pair before any is
    /// edited, as in Helix.
    fn surrounding_pairs(&mut self, character: char) -> Result<Option<Vec<SyntaxRange>>> {
        if self.refuse_read_only_surround() {
            return Ok(None);
        }
        let Some(pair) = existing_pair(character) else {
            self.action_failed(format!(
                "no surround pair for {character} · use ( [ {{ < \" ' ` or m"
            ));
            return Ok(None);
        };
        let buffer_id = self.active().buffer;
        let text = self.buffers[buffer_id].text();
        let end = text.len_chars();
        let half_open = self.active().selection_semantics() != SelectionSemantics::Runyte;
        let mut spans = Vec::new();
        for range in self.active().selection.ranges() {
            // A half-open range ends one past the character its caret is on.
            let caret = if half_open && range.head > range.anchor {
                range.head - 1
            } else {
                range.head
            };
            let requested = SyntaxRange::new(caret, (caret + 1).min(end))?;
            match enclosing_delimiter(
                self.syntax[buffer_id].as_ref(),
                text,
                &self.registry,
                requested,
                pair,
                SyntaxObjectPart::Around,
            )? {
                Some(span) => spans.push(span),
                None => {
                    self.action_failed(format!("no surrounding {}", pair_label(pair)));
                    return Ok(None);
                }
            }
        }
        Ok(Some(spans))
    }

    fn refuse_read_only_surround(&mut self) -> bool {
        let Some(reason) = self.active_buffer().read_only_reason() else {
            return false;
        };
        self.mode = Mode::Normal;
        self.action_failed(reason);
        true
    }
}
