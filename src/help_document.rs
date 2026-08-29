// SPDX-License-Identifier: MPL-2.0

//! Semantic text shared by Runyte's generated help pages.
//!
//! Help remains ordinary buffer text. This module only records what parts of
//! that text mean, in character offsets, so every frontend can colour the same
//! commands, keys, paths, links, headings, and technical literals without
//! parsing the rendered prose back into structure.

use crate::{
    syntax::{Scope, Span},
    text::Offset,
};

/// A semantic role in a generated help document.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HelpRole {
    Heading,
    Command,
    KeyBinding,
    FilePath,
    // No current built-in help text contains a URL, but the schema reserves
    // and tests the role so adding one cannot silently fall back to plain text.
    #[allow(dead_code)]
    WebLink,
    Code,
    Delimiter,
}

impl HelpRole {
    fn scope(self) -> Scope {
        let name = match self {
            Self::Heading => "markup.heading",
            Self::Command => "function",
            Self::KeyBinding => "keyword",
            Self::FilePath => "string",
            Self::WebLink => "markup.link.url",
            Self::Code => "markup.raw",
            Self::Delimiter => "punctuation",
        };
        Scope::named(name).expect("help roles use registered syntax scopes")
    }
}

/// Plain buffer text together with its non-overlapping semantic colour spans.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HelpDocument {
    text: String,
    spans: Vec<Span>,
}

impl HelpDocument {
    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn spans(&self) -> &[Span] {
        &self.spans
    }
}

/// Highlight spans intersecting `[from, to)`, clipped to that range.
pub(crate) fn clip_spans(spans: &[Span], from: Offset, to: Offset) -> Vec<Span> {
    if from >= to {
        return Vec::new();
    }
    spans
        .iter()
        .skip_while(|span| span.to <= from)
        .take_while(|span| span.from < to)
        .filter_map(|span| {
            let span = Span {
                from: span.from.max(from),
                to: span.to.min(to),
                scope: span.scope,
            };
            (span.from < span.to).then_some(span)
        })
        .collect()
}

/// Builds a help document while keeping roles aligned with Unicode character
/// offsets rather than UTF-8 bytes.
#[derive(Default)]
pub struct HelpDocumentWriter {
    text: String,
    roles: Vec<Option<HelpRole>>,
}

impl HelpDocumentWriter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn write(&mut self, text: &str) {
        self.text.push_str(text);
        self.roles.extend(text.chars().map(|_| None));
    }

    /// Writes prose whose visible backticks are explicit technical markers.
    /// The delimiters stay in the buffer and receive their own punctuation
    /// role; their contents receive `Code` unless a producer later applies a
    /// more specific role to that text.
    pub fn write_prose(&mut self, text: &str) {
        let start = self.roles.len();
        self.write(text);
        let mut open = None;
        for (relative, character) in text.chars().enumerate() {
            if character != '`' {
                continue;
            }
            let index = start + relative;
            self.roles[index] = Some(HelpRole::Delimiter);
            if let Some(from) = open.take() {
                self.roles[from + 1..index].fill(Some(HelpRole::Code));
            } else {
                open = Some(index);
            }
        }
    }

    /// Applies an explicitly declared role to every exact occurrence in the
    /// text written since `from`. Matches must begin and end at character
    /// boundaries. Later, more specific declarations replace earlier roles.
    pub fn mark_since(&mut self, from: usize, needle: &str, role: HelpRole) {
        if needle.is_empty() || from >= self.roles.len() {
            return;
        }
        let byte_from = char_to_byte(&self.text, from);
        let tail = &self.text[byte_from..];
        for (relative_byte, _) in tail.match_indices(needle) {
            let start = from + tail[..relative_byte].chars().count();
            let end = start + needle.chars().count();
            self.roles[start..end].fill(Some(role));
        }
    }

    /// Like [`Self::mark_since`], but ignores occurrences embedded in an
    /// ASCII word. This is useful for explicitly declared single-key labels:
    /// `v` is a key in "press v", but not in "move".
    pub fn mark_token_since(&mut self, from: usize, needle: &str, role: HelpRole) {
        if needle.is_empty() || from >= self.roles.len() {
            return;
        }
        let characters = self.text.chars().collect::<Vec<_>>();
        let needle_characters = needle.chars().count();
        let byte_from = char_to_byte(&self.text, from);
        let tail = &self.text[byte_from..];
        for (relative_byte, _) in tail.match_indices(needle) {
            let start = from + tail[..relative_byte].chars().count();
            let end = start + needle_characters;
            let embedded_left = start
                .checked_sub(1)
                .and_then(|index| characters.get(index))
                .is_some_and(|character| character.is_ascii_alphanumeric() || *character == '_');
            let embedded_right = characters
                .get(end)
                .is_some_and(|character| character.is_ascii_alphanumeric() || *character == '_');
            if !embedded_left && !embedded_right {
                self.roles[start..end].fill(Some(role));
            }
        }
    }

    pub fn finish(self) -> HelpDocument {
        let mut spans = Vec::new();
        let mut start = 0;
        while start < self.roles.len() {
            let Some(role) = self.roles[start] else {
                start += 1;
                continue;
            };
            let mut end = start + 1;
            while end < self.roles.len() && self.roles[end] == Some(role) {
                end += 1;
            }
            spans.push(Span {
                from: start,
                to: end,
                scope: role.scope(),
            });
            start = end;
        }
        HelpDocument {
            text: self.text,
            spans,
        }
    }
}

fn char_to_byte(text: &str, offset: usize) -> usize {
    text.char_indices()
        .nth(offset)
        .map_or(text.len(), |(byte, _)| byte)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roles_preserve_text_and_use_character_offsets() {
        let mut writer = HelpDocumentWriter::new();
        writer.write("λ ");
        writer.write(":help");
        writer.mark_since(2, ":help", HelpRole::Command);
        writer.write(" ");
        writer.write("https://runyte.dev");
        writer.mark_since(8, "https://runyte.dev", HelpRole::WebLink);
        let document = writer.finish();

        assert_eq!(document.text(), "λ :help https://runyte.dev");
        assert_eq!(document.spans()[0].from, 2);
        assert_eq!(document.spans()[0].to, 7);
        assert_eq!(document.spans()[0].scope.name(), "function");
        assert_eq!(document.spans()[1].scope.name(), "markup.link.url");
    }

    #[test]
    fn visible_backticks_keep_punctuation_around_code() {
        let mut writer = HelpDocumentWriter::new();
        writer.write_prose("Use `editor.mouse` with `:help`.");
        let start = 0;
        writer.mark_since(start, ":help", HelpRole::Command);
        let document = writer.finish();

        assert_eq!(document.text(), "Use `editor.mouse` with `:help`.");
        let roles = document
            .spans()
            .iter()
            .map(|span| (span.from, span.to, span.scope.name()))
            .collect::<Vec<_>>();
        assert!(roles.contains(&(4, 5, "punctuation")));
        assert!(roles.contains(&(5, 17, "markup.raw")));
        assert!(roles.contains(&(25, 30, "function")));
    }

    #[test]
    fn viewport_queries_clip_sorted_non_overlapping_spans() {
        let mut writer = HelpDocumentWriter::new();
        writer.write("Heading /tmp/file");
        writer.mark_since(0, "Heading", HelpRole::Heading);
        writer.mark_since(0, "/tmp/file", HelpRole::FilePath);
        let document = writer.finish();

        let clipped = clip_spans(document.spans(), 3, 12);
        assert_eq!(clipped.len(), 2);
        assert_eq!((clipped[0].from, clipped[0].to), (3, 7));
        assert_eq!((clipped[1].from, clipped[1].to), (8, 12));
    }
}
