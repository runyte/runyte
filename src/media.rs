// SPDX-License-Identifier: MPL-2.0

//! Native media classification. Media projections never own an editable file.

use std::path::Path;

pub fn supported(path: &Path) -> bool {
    path.extension()
        .and_then(|value| value.to_str())
        .is_some_and(|value| {
            matches!(
                value.to_ascii_lowercase().as_str(),
                "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "pdf"
            )
        })
}

/// Presentation-only operations emitted by the shared command registry.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ViewAction {
    ZoomIn,
    ZoomOut,
    Fit,
    ActualSize,
    Center,
    PanLeft,
    PanRight,
    PanUp,
    PanDown,
    CopySelection,
    ClearSelection,
    SelectAll,
    BeginSelection,
    ExtendLeft,
    ExtendRight,
    ExtendUp,
    ExtendDown,
}

#[derive(Clone, Debug)]
pub struct ViewRequest {
    pub pane: usize,
    pub path: std::path::PathBuf,
    pub page: usize,
    pub action: ViewAction,
}
