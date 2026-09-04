// SPDX-License-Identifier: MPL-2.0

//! Path and word-completion tokenization, ordering, and candidate helpers.

// Application-module dependencies:
use super::{Buffer, Completion, HashSet, Offset, Ordering, is_word, previous_offset};
/// The characters of the row a listing name is shown as: the name, followed
/// by the separator that marks a directory.
///
/// Comparing these against a row already kept costs nothing but the
/// comparison. A path popup in a large directory decides against far more
/// names than it keeps, and building the row for each of those only to drop
/// it is most of what completing a path in such a directory would cost.
pub(super) fn row_characters(
    name: &str,
    is_directory: bool,
    separator: char,
) -> impl Iterator<Item = char> + Clone {
    name.chars().chain(is_directory.then_some(separator))
}

/// Whether `name`'s row sorts at or after `row`, and so cannot displace it.
///
/// Character order and the byte order [`str`] compares by agree, because
/// UTF-8 keeps code points in order.
pub(super) fn row_is_not_before(
    name: &str,
    is_directory: bool,
    separator: char,
    row: &str,
) -> bool {
    row_characters(name, is_directory, separator).cmp(row.chars()) != Ordering::Less
}

/// The same question for the palette's order, which puts directories first
/// and compares rows without regard to case before falling back to the exact
/// spelling.
pub(super) fn hint_is_not_before(
    name: &str,
    is_directory: bool,
    separator: char,
    row: &(bool, String, String),
) -> bool {
    let (row_is_file, folded, exact) = row;
    let ordering = (!is_directory)
        .cmp(row_is_file)
        .then_with(|| compare_folded(name, is_directory, separator, folded))
        .then_with(|| row_characters(name, is_directory, separator).cmp(exact.chars()));
    ordering != Ordering::Less
}

/// Orders `name`'s row against an already-lowercased `folded` row, as
/// comparing their lowercased spellings would.
///
/// The ASCII case is answered from bytes because `char::to_lowercase`
/// consults the Unicode case tables for every character it is handed, and
/// this comparison is what a large directory spends most of its time on:
/// it runs once per entry, while the work it avoids runs a few hundred times.
pub(super) fn compare_folded(
    name: &str,
    is_directory: bool,
    separator: char,
    folded: &str,
) -> Ordering {
    if name.is_ascii() && folded.is_ascii() && separator.is_ascii() {
        return name
            .bytes()
            .chain(is_directory.then_some(separator as u8))
            .map(|byte| byte.to_ascii_lowercase())
            .cmp(folded.bytes());
    }
    row_characters(name, is_directory, separator)
        .flat_map(char::to_lowercase)
        .cmp(folded.chars())
}

pub(super) fn is_path_token_boundary(character: char) -> bool {
    character.is_whitespace()
        || matches!(
            character,
            '"' | '\'' | '`' | '<' | '>' | '|' | ';' | ',' | '(' | ')' | '[' | ']' | '{' | '}'
        )
}

pub(super) fn path_token_before(buffer: &Buffer, head: Offset) -> String {
    let row = buffer.offset_to_row(head);
    let start = buffer.line_to_offset(row);
    let line = buffer.slice(start, head);
    let token_start = line
        .char_indices()
        .rev()
        .find_map(|(index, character)| {
            is_path_token_boundary(character).then_some(index + character.len_utf8())
        })
        .unwrap_or(0);
    line[token_start..].to_owned()
}

/// Inclusive-start, exclusive-end bounds of the path-like token under a
/// caret, confined to one row. The token grammar is shared with path
/// completion so quotes and prose punctuation stop both features in the same
/// places, while separators remain part of the path.
pub(super) fn path_token_bounds(buffer: &Buffer, offset: Offset) -> Option<(Offset, Offset)> {
    let character = buffer.char_at(offset)?;
    if is_path_token_boundary(character) {
        return None;
    }
    let row = buffer.offset_to_row(offset);
    let row_start = buffer.line_to_offset(row);
    let row_end = row_start + buffer.line_len(row);
    let mut start = offset;
    while start > row_start {
        let previous = start - 1;
        if buffer.char_at(previous).is_none_or(is_path_token_boundary) {
            break;
        }
        start = previous;
    }
    let mut end = offset + 1;
    while end < row_end {
        if buffer.char_at(end).is_none_or(is_path_token_boundary) {
            break;
        }
        end += 1;
    }
    Some((start, end))
}

pub(super) fn is_word_completion_character(character: char) -> bool {
    character.is_alphanumeric() || character == '-'
}

/// The alphanumeric word fragment immediately before `head`. Interior
/// hyphens stay in the fragment, and a trailing hyphen is retained while the
/// next part of a hyphenated word is being typed.
pub(super) fn word_token_before(buffer: &Buffer, head: Offset) -> String {
    let row = buffer.offset_to_row(head);
    let start = buffer.line_to_offset(row);
    let line = buffer.slice(start, head);
    let mut valid_start = line.len();
    let mut hyphen_needs_left_side = false;
    let mut at_end = true;

    for (index, character) in line.char_indices().rev() {
        if character.is_alphanumeric() {
            valid_start = index;
            hyphen_needs_left_side = false;
            at_end = false;
        } else if character == '-'
            && !hyphen_needs_left_side
            && (valid_start < line.len() || at_end)
        {
            hyphen_needs_left_side = true;
            at_end = false;
        } else {
            break;
        }
    }
    line[valid_start..].to_owned()
}

/// Start of the identifier fragment a language completion should filter and,
/// absent a server-provided text edit, replace.
pub(super) fn language_completion_prefix_start(buffer: &Buffer, head: Offset) -> Offset {
    let mut start = head;
    while let Some(previous) = previous_offset(buffer, start) {
        if !buffer.char_at(previous).is_some_and(is_word) {
            break;
        }
        start = previous;
    }
    start
}

/// Appends words from `entries` that match `query` (already lowercased) and
/// have not already been offered, in the frequency order `entries` is sorted
/// by. `seen` tracks lowercased labels across both the active buffer and
/// every other buffer, so a word already offered from the active buffer is
/// not repeated from another one.
pub(super) fn push_matching_words(
    items: &mut Vec<Completion>,
    seen: &mut HashSet<String>,
    entries: &[(String, u32)],
    query: &str,
) {
    for (word, _) in entries {
        let lower = word.to_lowercase();
        if !lower.starts_with(query) {
            continue;
        }
        if seen.insert(lower) {
            items.push(Completion {
                label: word.clone(),
                filter_text: None,
                sort_text: None,
                detail: String::new(),
                kind: "word",
                insert: word.clone(),
                edit: None,
                additional: Vec::new(),
            });
        }
    }
}
