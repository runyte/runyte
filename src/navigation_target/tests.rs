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
