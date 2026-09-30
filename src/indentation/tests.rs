// SPDX-License-Identifier: MPL-2.0

#[test]
fn ordinary_backspace_stops_at_the_first_non_indent_character() {
    let mut visited = 0;
    let chars = "  x"
        .chars()
        .chain(std::iter::repeat_n('a', 1_000_000))
        .inspect(|_| visited += 1);
    assert_eq!(super::backspace_start(chars, 4), None);
    assert_eq!(visited, 3);
}
