// SPDX-License-Identifier: MPL-2.0

//! Shared geometry for a merge review's summary, body and pinned actions.

use crate::layout::Rect;
use unicode_width::UnicodeWidthStr;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct MergeReviewLayout {
    pub area: Rect,
    pub message: Rect,
    pub query: Rect,
    pub body: Rect,
    pub footer: Rect,
    pub stacked_footer: bool,
}

/// Reserve the actions first, then acknowledgment input, and leave at least
/// one body row whenever the terminal has room. Summary text yields space to
/// those interactive surfaces on a small terminal.
pub(crate) fn merge_review_layout(
    editor: Rect,
    message: &str,
    shows_query: bool,
) -> MergeReviewLayout {
    let width = (u32::from(editor.width) * 90 / 100)
        .max(28)
        .min(u32::from(editor.width)) as u16;
    let height = (u32::from(editor.height) * 85 / 100)
        .max(8)
        .min(u32::from(editor.height)) as u16;
    let area = Rect {
        x: editor.x + editor.width.saturating_sub(width) / 2,
        y: editor.y + editor.height.saturating_sub(height) / 2,
        width,
        height,
    };
    let inner = Rect {
        x: area.x.saturating_add(u16::from(width > 0)),
        y: area.y.saturating_add(u16::from(height > 0)),
        width: width.saturating_sub(2),
        height: height.saturating_sub(2),
    };
    let footer_height = if inner.width < 40 { 2 } else { 1 }.min(inner.height);
    let stacked_footer = footer_height == 2;
    let query_height = u16::from(shows_query && inner.height > footer_height);
    let remaining = inner.height.saturating_sub(footer_height + query_height);
    let summary_height =
        wrapped_rows(message, inner.width).min(usize::from(remaining.saturating_sub(1))) as u16;
    let message = Rect {
        height: summary_height,
        ..inner
    };
    let query = Rect {
        y: message.y + message.height,
        height: query_height,
        ..inner
    };
    let body = Rect {
        y: query.y + query.height,
        height: remaining.saturating_sub(summary_height),
        ..inner
    };
    let footer = Rect {
        y: body.y + body.height,
        height: footer_height,
        ..inner
    };
    MergeReviewLayout {
        area,
        message,
        query,
        body,
        footer,
        stacked_footer,
    }
}

fn wrapped_rows(message: &str, width: u16) -> usize {
    if message.is_empty() {
        return 0;
    }
    let width = usize::from(width.max(1));
    message
        .split('\n')
        .map(|line| {
            let mut rows = 1usize;
            let mut used = 0usize;
            for word in line.split_whitespace() {
                let cells = word.width();
                let separator = usize::from(used > 0);
                if separator + cells <= width.saturating_sub(used) {
                    used += separator + cells;
                } else {
                    rows += usize::from(used > 0) + cells.saturating_sub(1) / width;
                    used = cells.saturating_sub(1) % width + 1;
                }
            }
            rows
        })
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_geometry_keeps_sections_disjoint_and_actions_inside() {
        for height in 0..80 {
            for width in [0, 1, 2, 3, 8, 28, 80, u16::MAX] {
                for query in [false, true] {
                    let editor = Rect {
                        x: 0,
                        y: 0,
                        width,
                        height,
                    };
                    let view = merge_review_layout(
                        editor,
                        "Current: main\nOther: topic\nLong summary wraps into several rows",
                        query,
                    );
                    assert!(view.area.width <= width && view.area.height <= height);
                    assert_eq!(view.message.y + view.message.height, view.query.y);
                    assert_eq!(view.query.y + view.query.height, view.body.y);
                    assert_eq!(view.body.y + view.body.height, view.footer.y);
                    assert!(view.footer.y + view.footer.height <= view.area.y + view.area.height);
                    if height >= 3 {
                        assert!(view.footer.height >= 1);
                    }
                    if height >= 6 {
                        assert!(view.body.height >= 1);
                        assert_eq!(view.query.height, u16::from(query));
                    }
                }
            }
        }
    }
}
