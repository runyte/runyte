// SPDX-License-Identifier: MPL-2.0

//! Shared geometry for terminal proposal review and its input hit testing.

use crate::layout::Rect;

pub(crate) fn area(editor: Rect) -> Rect {
    let width = ((u32::from(editor.width) * 80 / 100) as u16)
        .max(80)
        .min(editor.width);
    let height = ((u32::from(editor.height) * 60 / 100) as u16)
        .max(22)
        .min(editor.height);
    Rect {
        x: editor.x + editor.width.saturating_sub(width) / 2,
        y: editor.y + editor.height.saturating_sub(height) / 2,
        width,
        height,
    }
}
