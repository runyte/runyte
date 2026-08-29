// SPDX-License-Identifier: MPL-2.0

//! The small, generated front page opened by `:about`.

use unicode_width::UnicodeWidthStr;

use crate::help_document::{HelpDocument, HelpDocumentWriter, HelpRole};

const LOGO: &str = include_str!("../logo/ascii/logo.txt");
const DESCRIPTION: &str = "A fast modal terminal editor with selection-first editing.";
const TAGLINE: &str = "Navigate. Select. Act.";
const HEADING: &str = "Getting around";
const FIRST_STEPS: &[(&str, &str)] = &[
    (":tutorial", "learn Runyte interactively"),
    ("Space ?", "help for the current view"),
    (":help", "open the general manual"),
    ("Space f", "find files, buffers, or terminals"),
    ("Space e", "explore the active directory"),
    ("Space b b", "list open buffers"),
    ("Alt-o | Alt-i", "move back and forth between buffers"),
    (":", "open the command palette"),
    (":q", "quit"),
];

/// Renders the about page as one block, no wider than its widest line.
///
/// The leading spaces left here are the block's own shape — the logo is drawn
/// with them, and the key table is centred against the sentence above it — so
/// they mean the same thing at every size. The margin around the block is not
/// here at all: the page asks to be centred with
/// [`ContentAlignment::CENTERED`](crate::content_alignment::ContentAlignment::CENTERED),
/// and the pane recomputes that space as it is resized. The result is still an
/// ordinary searchable, scrollable buffer, just like help.
pub fn render() -> String {
    render_document().text().to_owned()
}

pub(crate) fn render_document() -> HelpDocument {
    let logo = LOGO.lines().map(str::to_owned).collect::<Vec<_>>();
    let version = vec![format!("Runyte {}", env!("CARGO_PKG_VERSION"))];
    let description = vec![DESCRIPTION.to_owned()];
    let tagline = vec![TAGLINE.to_owned()];
    let heading = vec![HEADING.to_owned()];
    let steps = first_steps();
    let blocks = [&logo, &version, &description, &tagline, &heading, &steps];
    let width = blocks
        .iter()
        .map(|block| cells(block))
        .max()
        .unwrap_or_default();

    let mut text = String::new();
    push_block(&mut text, &logo, width);
    text.push('\n');
    push_block(&mut text, &version, width);
    text.push('\n');
    push_block(&mut text, &description, width);
    push_block(&mut text, &tagline, width);
    text.push('\n');
    push_block(&mut text, &heading, width);
    text.push('\n');
    push_block(&mut text, &steps, width);
    let mut document = HelpDocumentWriter::new();
    document.write(&text);
    for line in LOGO.lines().filter(|line| !line.is_empty()) {
        document.mark_since(0, line, HelpRole::Heading);
    }
    document.mark_since(0, &version[0], HelpRole::Heading);
    document.mark_since(0, HEADING, HelpRole::Heading);
    for (key, _) in FIRST_STEPS {
        let role = if key.starts_with(':') {
            HelpRole::Command
        } else {
            HelpRole::KeyBinding
        };
        document.mark_since(0, key, role);
    }
    document.finish()
}

/// The key table, each key padded to the widest so the separators line up.
fn first_steps() -> Vec<String> {
    let key_width = FIRST_STEPS
        .iter()
        .map(|(key, _)| key.width())
        .max()
        .unwrap_or_default();
    FIRST_STEPS
        .iter()
        .map(|(key, description)| format!("{key:<key_width$} · {description}"))
        .collect()
}

/// Centres one group of lines within `width`, moving them together.
///
/// The group is placed by its widest line, not line by line, so the logo keeps
/// its shape and the key table keeps its column. An empty line is left empty
/// rather than padded: trailing spaces would be the only thing on it, and they
/// are not part of what the page says.
fn push_block(text: &mut String, lines: &[String], width: usize) {
    let padding = width.saturating_sub(cells(lines)) / 2;
    for line in lines {
        if !line.is_empty() {
            text.extend(std::iter::repeat_n(' ', padding));
            text.push_str(line);
        }
        text.push('\n');
    }
}

fn cells(lines: &[String]) -> usize {
    lines
        .iter()
        .map(|line| line.width())
        .max()
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn about_contains_the_source_logo_version_and_first_steps() {
        let rendered = render();
        let width = DESCRIPTION.width();
        let logo_width = LOGO.lines().map(UnicodeWidthStr::width).max().unwrap();
        let prefix = " ".repeat((width - logo_width) / 2);
        for (rendered_line, source_line) in rendered.lines().zip(LOGO.lines()) {
            let expected = if source_line.is_empty() {
                String::new()
            } else {
                format!("{prefix}{source_line}")
            };
            assert_eq!(rendered_line, expected);
        }
        assert!(rendered.contains(&format!("Runyte {}", env!("CARGO_PKG_VERSION"))));
        assert!(rendered.contains("Getting around"));
        for row in [
            "Space ?       · help for the current view",
            ":help         · open the general manual",
            "Space f       · find files, buffers, or terminals",
            "Space e       · explore the active directory",
            "Space b b     · list open buffers",
            "Alt-o | Alt-i · move back and forth between buffers",
            ":             · open the command palette",
            ":q            · quit",
        ] {
            assert!(rendered.lines().any(|line| line.trim_start() == row));
        }
        assert!(rendered.contains("Navigate. Select. Act."));
    }

    /// The page carries no margin of its own: the widest line starts in the
    /// first column, and every other line is centred against it. Anything
    /// beyond that is the pane's, recomputed as it is resized.
    #[test]
    fn about_is_a_block_with_no_margin_around_it() {
        let rendered = render();
        let description = rendered
            .lines()
            .find(|line| line.contains("fast modal"))
            .unwrap();
        assert_eq!(description, DESCRIPTION);

        let width = rendered.lines().map(UnicodeWidthStr::width).max().unwrap();
        assert_eq!(width, DESCRIPTION.width());
        let quit = rendered
            .lines()
            .find(|line| line.ends_with("quit"))
            .unwrap();
        let steps = cells(&first_steps());
        assert_eq!(
            quit.width() - quit.trim_start().width(),
            (width - steps) / 2
        );
    }

    #[test]
    fn about_assigns_one_schema_to_titles_commands_and_keys() {
        let rendered = render_document();
        let scopes = |needle: &str| {
            let from = rendered.text().find(needle).unwrap();
            let from = rendered.text()[..from].chars().count();
            rendered
                .spans()
                .iter()
                .find(|span| span.from <= from && span.to > from)
                .map(|span| span.scope.name())
        };

        let logo_line = LOGO.lines().find(|line| !line.is_empty()).unwrap();
        assert_eq!(scopes(logo_line), Some("markup.heading"));
        assert_eq!(scopes("Getting around"), Some("markup.heading"));
        assert_eq!(scopes(":tutorial"), Some("function"));
        assert_eq!(scopes("Space ?"), Some("keyword"));
    }
}
