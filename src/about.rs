// SPDX-License-Identifier: MPL-2.0

//! The small, generated front page opened by `:about`.

use unicode_width::UnicodeWidthStr;

use crate::help_document::{HelpDocument, HelpDocumentWriter, HelpRole};
use crate::keymap::{Keymap, default_keymap};

const LOGO: &str = include_str!("../logo/ascii/logo.txt");
const DESCRIPTION: &str = "A fast modal terminal editor for focused work.";
const HEADING: &str = "Getting around";
const FIRST_STEPS: &[(&str, &str)] = &[
    (":tutorial", "learn Runyte interactively"),
    (":help", "open the general manual"),
    ("{binding:Space ?}", "help for the current view"),
    ("{binding:Space e}", "explore the active directory"),
    ("{binding:Space n}", "list open buffers and terminals"),
    (
        "{binding:Space f}",
        "find anything (Tab to switch between files/content)",
    ),
    (
        "{binding:Space Space}",
        "session management (persistent mode)",
    ),
    ("Alt-o | Alt-i", "move back and forth between buffers"),
    ("{prefix:Ctrl-w} ...", "pane management commands"),
    (":terminal", "integrated terminal"),
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
    render_document_for(default_keymap())
}

pub(crate) fn render_document_for(keymap: &Keymap) -> HelpDocument {
    let logo = LOGO.lines().map(str::to_owned).collect::<Vec<_>>();
    let version = vec![format!("Runyte {}", env!("CARGO_PKG_VERSION"))];
    let description = vec![DESCRIPTION.to_owned()];
    let heading = vec![HEADING.to_owned()];
    let steps = first_steps(keymap);
    let blocks = [&logo, &version, &description, &heading, &steps];
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
    for ((key, _), rendered) in FIRST_STEPS.iter().zip(&steps) {
        let key = if key.starts_with(':') {
            (*key).to_owned()
        } else {
            rendered
                .split_once(" · ")
                .map_or_else(String::new, |(key, _)| key.trim_end().to_owned())
        };
        let role = if key.starts_with(':') {
            HelpRole::Command
        } else {
            HelpRole::KeyBinding
        };
        document.mark_since(0, &key, role);
    }
    document.finish()
}

/// The key table, each key padded to the widest so the separators line up.
fn first_steps(keymap: &Keymap) -> Vec<String> {
    let resolved = FIRST_STEPS
        .iter()
        .map(|(key, description)| {
            let key = if key.starts_with(':') {
                (*key).to_owned()
            } else {
                crate::key_spelling::resolve(key, keymap)
                    .expect("about key markers must resolve")
                    .text
            };
            (key, *description)
        })
        .collect::<Vec<_>>();
    let key_width = resolved
        .iter()
        .map(|(key, _)| key.width())
        .max()
        .unwrap_or_default();
    resolved
        .into_iter()
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
    fn authored_key_markers_are_complete_and_resolve_in_both_variants() {
        for (key, _) in FIRST_STEPS {
            crate::key_spelling::assert_authored_template(key);
        }
    }

    #[test]
    fn about_contains_the_source_logo_version_and_first_steps() {
        let rendered = render();
        let width = rendered.lines().map(UnicodeWidthStr::width).max().unwrap();
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
            ":tutorial     · learn Runyte interactively",
            ":help         · open the general manual",
            "Space ?       · help for the current view",
            "Space e       · explore the active directory",
            "Space n       · list open buffers and terminals",
            "Space f       · find anything (Tab to switch between files/content)",
            "Space Space   · session management (persistent mode)",
            "Alt-o | Alt-i · move back and forth between buffers",
            "Ctrl-w ...    · pane management commands",
            ":terminal     · integrated terminal",
            ":             · open the command palette",
            ":q            · quit",
        ] {
            assert!(
                rendered.lines().any(|line| line.trim_start() == row),
                "missing {row:?} in\n{rendered}"
            );
        }
        let rows = rendered
            .lines()
            .skip_while(|line| !line.contains("Getting around"))
            .filter_map(|line| line.split_once(" · "))
            .map(|(key, _)| key.trim().to_owned())
            .collect::<Vec<_>>();
        assert_eq!(
            rows,
            [
                ":tutorial",
                ":help",
                "Space ?",
                "Space e",
                "Space n",
                "Space f",
                "Space Space",
                "Alt-o | Alt-i",
                "Ctrl-w ...",
                ":terminal",
                ":",
                ":q",
            ]
        );
        assert!(rendered.contains(DESCRIPTION));
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
        let width = rendered.lines().map(UnicodeWidthStr::width).max().unwrap();
        assert_eq!(description.trim_start(), DESCRIPTION);
        assert_eq!(
            description.width() - DESCRIPTION.width(),
            (width - DESCRIPTION.width()) / 2
        );
        let quit = rendered
            .lines()
            .find(|line| line.ends_with("quit"))
            .unwrap();
        let steps = cells(&first_steps(default_keymap()));
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
        assert_eq!(scopes("Space f"), Some("keyword"));
        assert_eq!(scopes("Ctrl-w ..."), Some("keyword"));
        assert_eq!(scopes(":terminal"), Some("function"));
    }
}
