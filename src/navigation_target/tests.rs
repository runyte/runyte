// SPDX-License-Identifier: MPL-2.0

use super::*;

#[test]
fn inferred_links_keep_queries_fragments_and_balanced_punctuation() {
    for (line, target) in [
        (
            "界 [docs](https://example.com/a_(b)?q=a,b&x=2#part).",
            "https://example.com/a_(b)?q=a,b&x=2#part",
        ),
        (
            "See <http://localhost:8080/a%20b>.",
            "http://localhost:8080/a%20b",
        ),
        ("(www.example.com/path).", "www.example.com/path"),
        ("`HTTPS://example.com`", "HTTPS://example.com"),
        ("http://[::1]:8080/page", "http://[::1]:8080/page"),
    ] {
        let byte = line.find(target).unwrap();
        let start = line[..byte].chars().count();
        for offset in start..start + target.chars().count() {
            assert_eq!(
                under_cursor(line, offset).as_deref(),
                Some(target),
                "{line} at {offset}"
            );
        }
    }
}

#[test]
fn paths_and_empty_carets_preserve_the_path_token_grammar() {
    for (line, offset, expected) in [
        ("`src/file.rs`", 5, Some("src/file.rs")),
        ("/tmp/界.txt", 5, Some("/tmp/界.txt")),
        ("file.txt other", 8, None),
        ("(file.txt)", 0, None),
        ("", 0, None),
        ("x", 1, None),
    ] {
        assert_eq!(under_cursor(line, offset).as_deref(), expected);
    }
}

#[test]
fn inferred_links_trim_long_mixed_suffixes_and_preserve_balanced_delimiters() {
    let target = "https://example.test/({nested[a_(b)]})";
    let suffix = ").]!};?:".repeat(16_384);
    let line = format!("界 <{target}{suffix}>");
    let start = "界 <".chars().count();
    assert_eq!(under_cursor(&line, start).as_deref(), Some(target));
    assert_eq!(
        under_cursor(&line, start + target.chars().count() - 1).as_deref(),
        Some(target)
    );
    assert_eq!(under_cursor(&line, start + target.chars().count()), None);
    assert_eq!(under_cursor(&line, start - 1), None);
}

#[test]
fn punctuation_carets_try_later_prefixes_without_repeated_suffix_scans() {
    let line = format!("{}{}", "https://a,".repeat(16_384), ")".repeat(16_384));
    assert_eq!(under_cursor(&line, line.len() - 1), None);
    // Earlier unmatched closers can make the first candidate trim a balanced
    // closer belonging to a later candidate. That later candidate still wins.
    let line = "https://a)))https://b()";
    assert_eq!(
        under_cursor(line, line.len() - 1).as_deref(),
        Some("https://b()")
    );
}

#[test]
fn indexed_url_suffixes_match_character_by_character_trimming() {
    fn reference(line: &str, caret: usize) -> Option<String> {
        for (index, _) in line.char_indices() {
            let mut link = &line[index..];
            if !web_prefix(link)
                || (index > 0 && !is_path_boundary(line[..index].chars().next_back().unwrap()))
            {
                continue;
            }
            while let Some(last) = link.chars().next_back() {
                let unmatched = match last {
                    ')' => Some(('(', ')')),
                    ']' => Some(('[', ']')),
                    '}' => Some(('{', '}')),
                    _ => None,
                }
                .is_some_and(|(open, close)| {
                    link.matches(close).count() > link.matches(open).count()
                });
                if matches!(last, '.' | ',' | ';' | ':' | '!' | '?') || unmatched {
                    link = &link[..link.len() - last.len_utf8()];
                } else {
                    break;
                }
            }
            if caret >= index && caret < index + link.len() && web_url(link).is_some() {
                return Some(link.to_owned());
            }
        }
        None
    }
    let alphabet = ['(', ')', '[', ']', '{', '}', ',', '.'];
    for value in 0..8usize.pow(4) {
        let mut value = value;
        let mut middle = String::new();
        for _ in 0..4 {
            middle.push(alphabet[value % 8]);
            value /= 8;
        }
        for prefix in ["https://a", "https://a)))https://b", "http:///,https://b"] {
            let line = format!("{prefix}{middle})");
            for caret in [0, line.len() - 1] {
                if caret == 0 && prefix.starts_with("http:///") {
                    continue;
                }
                assert_eq!(
                    under_cursor(&line, caret),
                    reference(&line, caret),
                    "{line} at {caret}"
                );
            }
        }
    }
    let controlled = "https://a\u{1b},https://b";
    assert_eq!(
        under_cursor(controlled, controlled.len() - 1).as_deref(),
        Some("https://b")
    );
}

#[test]
fn terminal_path_punctuation_reaches_literal_first_resolution() {
    for (line, target) in [
        ("Changed src/file.rs, next", "src/file.rs,"),
        ("Changed (src/file.rs).", "src/file.rs)."),
        ("Changed src/file.rs,", "src/file.rs,"),
        ("Changed src/file.rs,other", "src/file.rs"),
    ] {
        let offset = line.find("src/").unwrap() + 4;
        assert_eq!(under_cursor(line, offset).as_deref(), Some(target));
    }
}

#[test]
fn only_web_addresses_use_the_browser_and_www_defaults_to_https() {
    for text in [
        "file.txt",
        "ftp://example.com",
        "javascript:alert(1)",
        "https://",
        "www.",
        "https://example.com\n",
        "http://example.com/\u{1b}",
    ] {
        assert_eq!(web_url(text), None, "{text}");
    }
    assert_eq!(
        web_url("www.example.com/a").as_deref(),
        Some("https://www.example.com/a")
    );
    assert_eq!(
        web_url("http://example.com/a?q=1#b").as_deref(),
        Some("http://example.com/a?q=1#b")
    );
}

#[test]
fn percent_decoding_leaves_malformed_escapes_and_invalid_utf8_as_written() {
    assert_eq!(percent_decode("my%20notes.md"), "my notes.md");
    assert_eq!(percent_decode("caf%C3%A9"), "café");
    assert_eq!(percent_decode("100%+1%zz%4"), "100%+1%zz%4");
    assert_eq!(percent_decode("bad%FF.md"), "bad%FF.md");
}
