// SPDX-License-Identifier: MPL-2.0

use super::*;

#[test]
fn literal_single_line_text_preserves_spaces_unicode_and_shell_syntax() {
    for value in [
        "cargo test",
        "  prompt with spaces  ",
        "é 界 e\u{301}",
        r"printf '\n'",
        "a; b && c | d $(e)",
    ] {
        assert_eq!(Text::new(value).unwrap().as_str(), value);
    }
}

#[test]
fn submission_controls_and_injected_paste_frames_are_rejected_whole() {
    for control in (0..=0x1f).chain(0x7f..=0x9f).chain([0x2028, 0x2029]) {
        let text = format!("safe{}suffix", char::from_u32(control).unwrap());
        assert_eq!(Text::new(&text), Err(TextError::ControlOrLineBreak));
    }
    for text in [
        "one\r\ntwo",
        "one\ntwo",
        "\x1b[201~danger\r",
        "\x1b[13u",
        "\x1bOM",
        "\x1b[200~text\x1b[201~",
    ] {
        assert_eq!(Text::new(text), Err(TextError::ControlOrLineBreak));
    }
}

#[test]
fn proposal_bounds_are_bytes_and_debug_does_not_print_content() {
    assert_eq!(Text::new(""), Err(TextError::Empty));
    assert_eq!(
        Text::new(&"x".repeat(MAX_TEXT_BYTES + 1)),
        Err(TextError::TooLong)
    );
    let unicode = "界".repeat(MAX_TEXT_BYTES / 3);
    assert_eq!(Text::new(&unicode).unwrap().as_str(), unicode);
    assert_eq!(Text::new(&(unicode + "界")), Err(TextError::TooLong));
    assert!(Text::new(&"x".repeat(MAX_TEXT_BYTES)).is_ok());
    let text = Text::new("private prompt").unwrap();
    assert!(!format!("{text:?}").contains("private prompt"));
}
