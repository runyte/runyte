// SPDX-License-Identifier: MPL-2.0

//! Unicode character-indexed editing primitives for interaction-line prompts.

pub(super) fn char_to_byte(value: &str, character_index: usize) -> usize {
    value
        .char_indices()
        .nth(character_index)
        .map_or(value.len(), |(byte, _)| byte)
}

pub(super) fn prompt_insert(value: &mut String, cursor: usize, character: char) {
    value.insert(char_to_byte(value, cursor), character);
}

pub(super) fn prompt_delete_range(value: &mut String, start: usize, end: usize) {
    let start = char_to_byte(value, start);
    let end = char_to_byte(value, end);
    value.replace_range(start..end, "");
}

/// Deletes the whole character before the cursor, never one code point of
/// an emoji sequence or a base without its marks.
pub(super) fn prompt_backspace(value: &mut String, cursor: &mut usize) {
    if *cursor == 0 {
        return;
    }
    let start = crate::grapheme::str_previous(value, *cursor);
    prompt_delete_range(value, start, *cursor);
    *cursor = start;
}

pub(super) fn prompt_delete(value: &mut String, cursor: usize) {
    if cursor < value.chars().count() {
        let end = crate::grapheme::str_next(value, cursor);
        prompt_delete_range(value, cursor, end);
    }
}

/// The cursor one whole character to the left.
pub(super) fn prompt_left(value: &str, cursor: usize) -> usize {
    crate::grapheme::str_previous(value, cursor)
}

/// The cursor one whole character to the right, stopping at the end.
pub(super) fn prompt_right(value: &str, cursor: usize) -> usize {
    crate::grapheme::str_next(value, cursor).min(value.chars().count())
}

pub(super) fn prompt_word_backward(value: &str, cursor: usize) -> usize {
    let characters = value.chars().collect::<Vec<_>>();
    let mut cursor = cursor.min(characters.len());
    while cursor > 0 && characters[cursor - 1].is_whitespace() {
        cursor -= 1;
    }
    while cursor > 0 && !characters[cursor - 1].is_whitespace() {
        cursor -= 1;
    }
    cursor
}

pub(super) fn prompt_word_forward(value: &str, cursor: usize) -> usize {
    let characters = value.chars().collect::<Vec<_>>();
    let mut cursor = cursor.min(characters.len());
    while cursor < characters.len() && !characters[cursor].is_whitespace() {
        cursor += 1;
    }
    while cursor < characters.len() && characters[cursor].is_whitespace() {
        cursor += 1;
    }
    cursor
}
