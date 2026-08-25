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

pub(super) fn prompt_backspace(value: &mut String, cursor: &mut usize) {
    if *cursor == 0 {
        return;
    }
    prompt_delete_range(value, *cursor - 1, *cursor);
    *cursor -= 1;
}

pub(super) fn prompt_delete(value: &mut String, cursor: usize) {
    if cursor < value.chars().count() {
        prompt_delete_range(value, cursor, cursor + 1);
    }
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
