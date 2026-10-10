// SPDX-License-Identifier: MPL-2.0
use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};
use std::sync::OnceLock;
use syntect::{highlighting::ThemeSet, parsing::SyntaxSet};

pub fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}
fn code(text: &str, language: &str) -> String {
    static SYNTAX: OnceLock<SyntaxSet> = OnceLock::new();
    static THEMES: OnceLock<ThemeSet> = OnceLock::new();
    let syntax = SYNTAX.get_or_init(SyntaxSet::load_defaults_newlines);
    let themes = THEMES.get_or_init(ThemeSet::load_defaults);
    let language = match language {
        "javascript" => "js",
        "typescript" => "ts",
        "python" => "py",
        "rust" => "rs",
        x => x,
    };
    let grammar = syntax
        .find_syntax_by_token(language)
        .unwrap_or_else(|| syntax.find_syntax_plain_text());
    syntect::html::highlighted_html_for_string(
        text,
        syntax,
        grammar,
        &themes.themes["InspiredGitHub"],
    )
    .unwrap_or_else(|_| format!("<pre>{}</pre>", escape(text)))
}
pub fn document(text: &str, language: &str) -> String {
    if language == "svg" {
        let error = match roxmltree::Document::parse(text) {
            Ok(doc) if doc.root_element().tag_name().name() == "svg" => None,
            Ok(_) => Some("expected an svg root element".to_owned()),
            Err(error) => Some(error.to_string()),
        };
        let body = if let Some(error) = error {
            format!(
                "<p>Invalid SVG: {}</p>{}",
                escape(&error),
                code(text, "xml")
            )
        } else {
            text.to_owned()
        };
        return format!(
            "<!doctype html><html><body style=\"margin:0;background:white\">{body}</body></html>"
        );
    }
    if language == "html" {
        return text.to_owned();
    }
    let body = match language {
        "markdown" => {
            let parser = Parser::new_ext(
                text,
                Options::ENABLE_TABLES
                    | Options::ENABLE_TASKLISTS
                    | Options::ENABLE_STRIKETHROUGH
                    | Options::ENABLE_GFM,
            );
            let mut events = Vec::new();
            let mut fence: Option<(String, String)> = None;
            for event in parser {
                match event {
                    Event::Start(Tag::CodeBlock(kind)) => {
                        let lang = match kind {
                            pulldown_cmark::CodeBlockKind::Fenced(s) => s.to_string(),
                            _ => String::new(),
                        };
                        fence = Some((lang, String::new()));
                    }
                    Event::End(TagEnd::CodeBlock) => {
                        if let Some((lang, text)) = fence.take() {
                            events.push(Event::Html(code(&text, &lang).into()));
                        }
                    }
                    Event::Text(text) if fence.is_some() => {
                        fence.as_mut().unwrap().1.push_str(&text)
                    }
                    // Raw Markdown HTML is text, never an active document element.
                    Event::Html(text) | Event::InlineHtml(text) => events.push(Event::Text(text)),
                    event => events.push(event),
                }
            }
            let mut result = String::new();
            pulldown_cmark::html::push_html(&mut result, events.into_iter());
            result
        }
        "json" => match serde_json::from_str::<serde_json::Value>(text) {
            Ok(value) => code(&serde_json::to_string_pretty(&value).unwrap(), "json"),
            Err(error) => format!(
                "<p class=diagnostic>Invalid JSON: {}</p>{}",
                escape(&error.to_string()),
                code(text, "json")
            ),
        },
        _ => code(text, language),
    };
    format!(
        "<!doctype html><html><head><style>{}</style></head><body class=markdown-body>{body}</body></html>",
        include_str!("../preview.css")
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn text_is_escaped() {
        let html = document("<script>alert(1)</script>&", "unknown");
        assert!(!html.contains("<script>"));
        assert!(html.contains("&lt;"));
    }
    #[test]
    fn yaml_preserves_structure() {
        let text = "# comment\na: &x [one, two]\nb: *x\n";
        let html = document(text, "yaml");
        let doc = blitz_html::HtmlDocument::from_html(&html, Default::default());
        let pre = doc.query_selector("pre").unwrap().unwrap();
        assert_eq!(doc.get_node(pre).unwrap().text_content(), text);
    }
    #[test]
    fn invalid_json_falls_back() {
        let html = document("{broken}", "json");
        assert!(html.contains("Invalid JSON:"));
        let doc = blitz_html::HtmlDocument::from_html(&html, Default::default());
        let pre = doc.query_selector("pre").unwrap().unwrap();
        assert_eq!(doc.get_node(pre).unwrap().text_content(), "{broken}");
    }
    #[test]
    fn valid_json_is_pretty() {
        let html = document("{\"a\":1}", "json");
        let doc = blitz_html::HtmlDocument::from_html(&html, Default::default());
        let pre = doc.query_selector("pre").unwrap().unwrap();
        assert_eq!(
            doc.get_node(pre).unwrap().text_content(),
            "{\n  \"a\": 1\n}"
        );
        assert!(!html.contains("Invalid JSON"));
    }
    #[test]
    fn malformed_svg_keeps_source_and_diagnostic() {
        let html = document("<svg><path", "svg");
        assert!(html.contains("Invalid SVG:"));
        let doc = blitz_html::HtmlDocument::from_html(&html, Default::default());
        let pre = doc.query_selector("pre").unwrap().unwrap();
        assert_eq!(doc.get_node(pre).unwrap().text_content(), "<svg><path");
    }
    #[test]
    fn html_is_document() {
        assert_eq!(document("<h1>hello</h1>", "html"), "<h1>hello</h1>");
    }
    #[test]
    fn markdown_features() {
        let html = document(
            "# Title\n\n- [x] task\n\n|a|b|\n|-|-|\n|1|2|\n\n<script>x</script>",
            "markdown",
        );
        for expected in ["<h1>", "<table>", "checkbox", "&lt;script&gt;"] {
            assert!(html.contains(expected), "{expected}");
        }
    }
}
