// SPDX-License-Identifier: MPL-2.0

//! Immutable committed comparisons and their searchable file-list projection.

use super::{FileComparison, LineStats};
use std::path::PathBuf;
use unicode_width::UnicodeWidthStr;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ComparisonTarget {
    Branch { reference: String, label: String },
    Worktree(PathBuf),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RevisionFile {
    pub left: Option<PathBuf>,
    pub right: Option<PathBuf>,
    pub left_object: String,
    pub right_object: String,
    pub left_mode: String,
    pub right_mode: String,
    pub stats: Option<LineStats>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RevisionComparison {
    pub target: ComparisonTarget,
    pub left_label: String,
    pub right_label: String,
    pub left_oid: String,
    pub right_oid: String,
    pub files: Vec<RevisionFile>,
}

#[derive(Clone, Debug)]
pub enum RevisionFileView {
    Patch(String),
    Split(FileComparison),
}

impl RevisionComparison {
    /// Small immutable endpoint capture for one file request.
    pub fn endpoints(&self) -> Self {
        Self {
            target: self.target.clone(),
            left_label: self.left_label.clone(),
            right_label: self.right_label.clone(),
            left_oid: self.left_oid.clone(),
            right_oid: self.right_oid.clone(),
            files: Vec::new(),
        }
    }

    /// Every projection has four header rows, so resizing preserves row identity.
    pub fn render(&self, width: usize) -> String {
        let names: Vec<_> = self
            .files
            .iter()
            .map(|file| {
                let name = |path: &Option<PathBuf>| {
                    path.as_deref()
                        .map(super::display_path)
                        .unwrap_or_else(|| "—".into())
                };
                (name(&file.left), name(&file.right))
            })
            .collect();
        let left_width = names
            .iter()
            .map(|(left, _)| left.width())
            .chain([self.left_label.width()])
            .max()
            .unwrap_or(0);
        let right_width = names
            .iter()
            .map(|(_, right)| right.width())
            .chain([self.right_label.width()])
            .max()
            .unwrap_or(0);
        let wide = left_width + right_width + 24 <= width;
        let total = self
            .files
            .iter()
            .filter_map(|file| file.stats)
            .fold(LineStats::default(), |sum, count| sum.sum(count));
        let pad = |text: &str, columns: usize| {
            format!("{text}{}", " ".repeat(columns.saturating_sub(text.width())))
        };
        let mut text = format!(
            "Comparing {} ({}) → {} ({}) · committed changes\n{} files · +{} -{}\n\n",
            self.left_label,
            &self.left_oid[..8],
            self.right_label,
            &self.right_oid[..8],
            self.files.len(),
            total.added,
            total.removed
        );
        if wide {
            text.push_str(&format!(
                "{}  {}  Changes\n",
                pad(&self.left_label, left_width),
                pad(&self.right_label, right_width)
            ));
        } else {
            text.push_str("File · Changes\n");
        }
        for (file, (left, right)) in self.files.iter().zip(names) {
            let changes = match file.stats {
                None => "binary".into(),
                Some(LineStats {
                    added: 0,
                    removed: 0,
                }) if file.left.is_none() => "added (empty)".into(),
                Some(LineStats {
                    added: 0,
                    removed: 0,
                }) if file.right.is_none() => "deleted (empty)".into(),
                Some(LineStats {
                    added: 0,
                    removed: 0,
                }) if file.left_mode != file.right_mode => "mode changed".into(),
                Some(LineStats {
                    added: 0,
                    removed: 0,
                }) => "renamed".into(),
                Some(count) => format!("+{} -{}", count.added, count.removed),
            };
            if wide {
                text.push_str(&format!(
                    "{}  {}  {changes}\n",
                    pad(&left, left_width),
                    pad(&right, right_width)
                ));
            } else {
                let name = if left == right {
                    left
                } else {
                    format!("{left} → {right}")
                };
                text.push_str(&format!("{name}  {changes}\n"));
            }
        }
        if self.files.is_empty() {
            text.push_str("No committed differences.\n");
        }
        text
    }
}
