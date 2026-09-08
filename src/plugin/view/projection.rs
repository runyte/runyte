// SPDX-License-Identifier: MPL-2.0
use super::{Error, MAX_PROJECTION_BYTES, MAX_RETAINED_BYTES, Model, Role, cancelled, limited};
use crate::syntax::{Scope, Span};
use std::{
    collections::BTreeMap,
    sync::{Arc, atomic::AtomicBool},
};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

#[derive(Clone, Debug)]
pub struct ProjectedRow {
    pub id: String,
    pub from: usize,
    pub to: usize,
    pub line: usize,
}

#[derive(Debug)]
pub struct Projection {
    /// Taken by the buffer preparation worker before this projection is retained.
    pub text: String,
    pub spans: Vec<Span>,
    pub rows: Vec<ProjectedRow>,
    pub row_by_id: BTreeMap<String, usize>,
    pub line_rows: Vec<Option<usize>>,
}

#[derive(Debug)]
pub struct PreparedModel {
    pub model: Arc<Model>,
    pub encoded: Arc<str>,
    pub projection: Arc<Projection>,
    /// Includes installed rope storage and the installed copy of semantic spans.
    pub charge: usize,
}
impl PreparedModel {
    pub fn build(model: Model, cancel: &AtomicBool) -> Result<Self, Error> {
        Self::build_shared(Arc::new(model), cancel)
    }
    pub fn build_shared(model: Arc<Model>, cancel: &AtomicBool) -> Result<Self, Error> {
        cancelled(cancel)?;
        let encoded: Arc<str> = model.checked_json()?.into();
        cancelled(cancel)?;
        let projection = Projection::build(&model, cancel)?;
        let charge = model.payload_bytes()
            + encoded.len()
            + projection.text.len()
            + projection.spans.capacity() * std::mem::size_of::<Span>() * 2
            + projection.rows.capacity() * std::mem::size_of::<ProjectedRow>()
            + projection
                .rows
                .iter()
                .map(|row| row.id.capacity())
                .sum::<usize>()
            + projection
                .row_by_id
                .keys()
                .map(|id| id.capacity() + 128)
                .sum::<usize>()
            + projection.line_rows.capacity() * std::mem::size_of::<Option<usize>>();
        if charge > MAX_RETAINED_BYTES {
            return Err(limited("Prepared view storage limit exceeded"));
        }
        cancelled(cancel)?;
        Ok(Self {
            model,
            encoded,
            projection: Arc::new(projection),
            charge,
        })
    }
}

struct Builder {
    projection: Projection,
    chars: usize,
}
impl Builder {
    fn new(model: &Model) -> Self {
        let block_lines: usize = [&model.status, &model.detail, &model.preview]
            .into_iter()
            .flatten()
            .map(|block| block.text.split('\n').count() + 1)
            .sum();
        let spans = model
            .rows
            .iter()
            .map(|row| {
                if row.cells.is_empty() {
                    usize::from(row.role != Role::Ordinary)
                } else {
                    row.cells
                        .iter()
                        .filter(|cell| cell.role != Role::Ordinary)
                        .count()
                }
            })
            .sum::<usize>()
            + block_lines
            + model.columns.len();
        Self {
            projection: Projection {
                text: String::new(),
                spans: Vec::with_capacity(spans),
                rows: Vec::with_capacity(model.rows.len()),
                row_by_id: BTreeMap::new(),
                line_rows: Vec::with_capacity(model.rows.len() + block_lines + 2),
            },
            chars: 0,
        }
    }
    fn push(&mut self, text: &str, role: Role) -> Result<(), Error> {
        if self.projection.text.len().saturating_add(text.len()) > MAX_PROJECTION_BYTES {
            return Err(limited("Generated view text exceeds limit"));
        }
        let from = self.chars;
        self.chars += text.chars().count();
        self.projection.text.push_str(text);
        if self.chars > from
            && let Some(scope) = role_scope(role)
        {
            self.projection.spans.push(Span {
                from,
                to: self.chars,
                scope,
            });
        }
        Ok(())
    }
    fn newline(&mut self, row: Option<usize>) -> Result<(), Error> {
        self.push("\n", Role::Ordinary)?;
        self.projection.line_rows.push(row);
        Ok(())
    }
    fn block(&mut self, block: &super::Block) -> Result<(), Error> {
        for line in block.text.split('\n') {
            self.push(line, block.role)?;
            self.newline(None)?;
        }
        Ok(())
    }
}
impl Projection {
    /// Maps old row positions to retained identities; deletion chooses the nearest
    /// survivor in old order, preferring the predecessor at equal distance.
    pub fn remap_from(&self, old: &Self) -> Vec<usize> {
        let matched: Vec<_> = old
            .rows
            .iter()
            .map(|row| self.row_by_id.get(&row.id).copied())
            .collect();
        let mut next = vec![None; matched.len()];
        let mut nearest = None;
        for index in (0..matched.len()).rev() {
            if let Some(row) = matched[index] {
                nearest = Some((index, row));
            }
            next[index] = nearest;
        }
        let mut previous = None;
        matched
            .iter()
            .enumerate()
            .map(|(index, exact)| {
                if let Some(row) = exact {
                    previous = Some((index, *row));
                    return *row;
                }
                match (previous, next[index]) {
                    (Some((left, row)), Some((right, _))) if index - left <= right - index => row,
                    (_, Some((_, row))) | (Some((_, row)), None) => row,
                    (None, None) => index.min(self.rows.len().saturating_sub(1)),
                }
            })
            .collect()
    }

    fn build(model: &Model, cancel: &AtomicBool) -> Result<Self, Error> {
        cancelled(cancel)?;
        let mut builder = Builder::new(model);
        if let Some(status) = &model.status {
            builder.block(status)?;
        }
        let mut widths: Vec<_> = model
            .columns
            .iter()
            .map(|column| column.label.width().min(32))
            .collect();
        for row in &model.rows {
            cancelled(cancel)?;
            for (width, cell) in widths.iter_mut().zip(&row.cells) {
                *width = (*width).max(cell.text.width().min(32));
            }
        }
        if !widths.is_empty() {
            for (index, column) in model.columns.iter().enumerate() {
                let text = column_text(&column.label, widths[index]);
                builder.push(&text, Role::Heading)?;
                if index + 1 != widths.len() {
                    builder.push("  ", Role::Ordinary)?;
                }
            }
            builder.newline(None)?;
        }
        for (index, row) in model.rows.iter().enumerate() {
            cancelled(cancel)?;
            let from = builder.chars;
            let line = builder.projection.line_rows.len();
            if row.cells.is_empty() {
                builder.push(&row.text, row.role)?;
            } else {
                for (column, cell) in row.cells.iter().enumerate() {
                    let text = column_text(&cell.text, widths[column]);
                    builder.push(&text, cell.role)?;
                    if column + 1 != row.cells.len() {
                        builder.push("  ", Role::Ordinary)?;
                    }
                }
            }
            builder.projection.row_by_id.insert(row.id.clone(), index);
            builder.projection.rows.push(ProjectedRow {
                id: row.id.clone(),
                from,
                to: builder.chars,
                line,
            });
            builder.newline(Some(index))?;
        }
        if let Some(detail) = &model.detail {
            builder.push("Detail", Role::Heading)?;
            builder.newline(None)?;
            builder.block(detail)?;
        }
        if let Some(preview) = &model.preview {
            builder.push("Preview", Role::Heading)?;
            builder.newline(None)?;
            builder.block(preview)?;
        }
        // Text's trailing empty line is not an actionable row.
        builder.projection.line_rows.push(None);
        Ok(builder.projection)
    }
}

fn role_scope(role: Role) -> Option<Scope> {
    Scope::named(match role {
        Role::Ordinary => return None,
        Role::Muted => "comment",
        Role::Heading => "markup.heading",
        Role::Warning => "diagnostic.warning",
        Role::Error => "diagnostic.error",
    })
}
fn column_text(value: &str, width: usize) -> String {
    let full = value.width();
    let mut text = String::new();
    let mut used = 0;
    let truncated = full > width;
    let available = width.saturating_sub(usize::from(truncated));
    for grapheme in value.graphemes(true) {
        let cells = grapheme.width();
        if used + cells > available {
            break;
        }
        text.push_str(grapheme);
        used += cells;
    }
    if truncated && width > 0 {
        text.push('…');
        used += 1;
    }
    text.extend(std::iter::repeat_n(' ', width.saturating_sub(used)));
    text
}
