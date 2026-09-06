// SPDX-License-Identifier: MPL-2.0

//! Shared session-strip labels and cell bounds for drawing and pointer input.

use std::{ops::Range, path::PathBuf};

use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use crate::snapshot::SessionStripSnapshot;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PreparedSessionStrip {
    pub snapshot: SessionStripSnapshot,
    /// Catalog identities captured with the labels, never resolved by name or number.
    pub targets: Vec<PathBuf>,
}

#[derive(Default)]
pub(crate) struct SessionStripLayout {
    pub entries: Vec<SessionStripLabel>,
    pub trailing: Option<String>,
}

pub(crate) struct SessionStripLabel {
    pub index: usize,
    pub text: String,
    pub cells: Range<usize>,
}

impl SessionStripSnapshot {
    pub(crate) fn layout(&self, width: u16) -> SessionStripLayout {
        let labels = self
            .entries
            .iter()
            .map(|entry| {
                let number = entry
                    .number
                    .map_or_else(String::new, |number| format!("{number} "));
                // Every entry carries a marker, quiet included, so a session's
                // width never changes with its state and the names stay put.
                format!(
                    " {number}{} {} ",
                    entry.name,
                    if entry.health_unknown {
                        '?'
                    } else if entry.bell {
                        '!'
                    } else if entry.unread {
                        '+'
                    } else {
                        '·'
                    }
                )
            })
            .collect::<Vec<_>>();
        let widths = labels
            .iter()
            .map(|label| UnicodeWidthStr::width(label.as_str()))
            .collect::<Vec<_>>();
        let Some(current) = self
            .entries
            .iter()
            .position(|entry| entry.current)
            .or_else(|| (!labels.is_empty()).then_some(0))
        else {
            return SessionStripLayout::default();
        };
        let width = usize::from(width);
        // The count of omitted entries elides rather than adds: `+` is an entry's
        // own unread marker, so a strip ending in `+3` would read as a session
        // named 3 with new output. The reserve measures the exact string the
        // trailing span will draw, in cells rather than bytes.
        let omitted = |hidden: usize| format!(" …{hidden}");
        let reserve = if labels.len() > 1 {
            UnicodeWidthStr::width(omitted(labels.len() - 1).as_str())
        } else {
            0
        };
        let all_fit = widths.iter().sum::<usize>() <= width;
        // Identity wins on tiny terminals. An overflow count cannot replace the
        // current session, and padding must not consume its only available cell.
        let narrow = !all_fit && widths[current] + reserve > width;
        let budget = if all_fit {
            width
        } else {
            width.saturating_sub(reserve)
        };
        let mut start = if all_fit { 0 } else { current };
        let mut end = if all_fit { labels.len() } else { current + 1 };
        let mut used = widths[current];
        if !all_fit && !narrow {
            while start > 0 && used + widths[start - 1] <= budget {
                start -= 1;
                used += widths[start];
            }
            while end < labels.len() && used + widths[end] <= budget {
                used += widths[end];
                end += 1;
            }
        }
        let hidden = labels.len() - (end - start);

        let mut column = 0;
        let entries = (start..end)
            .map(|index| {
                let text = if narrow {
                    // Clip whole graphemes so the drawn and clickable cells agree,
                    // including combining marks and multi-codepoint emoji.
                    let mut text = String::new();
                    let mut used = 0;
                    for grapheme in labels[index].trim_start().graphemes(true) {
                        let cells = UnicodeWidthStr::width(grapheme);
                        if used + cells > width {
                            break;
                        }
                        text.push_str(grapheme);
                        used += cells;
                    }
                    text
                } else {
                    labels[index].clone()
                };
                let start = column;
                column += UnicodeWidthStr::width(text.as_str());
                SessionStripLabel {
                    index,
                    text,
                    cells: start..column,
                }
            })
            .collect();
        SessionStripLayout {
            entries,
            trailing: (hidden > 0 && !narrow).then(|| omitted(hidden)),
        }
    }
}
