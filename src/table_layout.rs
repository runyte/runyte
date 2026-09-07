// SPDX-License-Identifier: MPL-2.0

//! Pane-width geometry over stable character ranges in a rendered table.
//! This module owns no editor state or terminal styles. Padding and repeated
//! rules are presentation; only cell text maps back to selectable characters.

use crate::{
    buffer::Buffer,
    wrap::{self, Segment},
};
use std::{
    ops::Range,
    sync::{Arc, Mutex},
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TableRow {
    pub line: usize,
    pub cells: Vec<Range<usize>>,
    pub widths: Arc<Columns>,
    pub rule: bool,
    pub separator: bool,
}

/// Intrinsic column measurements shared by every row of one table. Tabbed
/// cells are measured once per tab width, rather than once for every row.
#[derive(Debug)]
pub(crate) struct Columns {
    natural: Vec<usize>,
    tabbed: Vec<(usize, String)>,
    measured: Mutex<Vec<MeasuredColumns>>,
}

#[derive(Debug)]
struct MeasuredColumns {
    tab: usize,
    widths: Arc<Vec<usize>>,
}

impl PartialEq for Columns {
    fn eq(&self, other: &Self) -> bool {
        self.natural == other.natural && self.tabbed == other.tabbed
    }
}
impl Eq for Columns {}

impl Columns {
    pub fn new(natural: Vec<usize>, tabbed: Vec<(usize, String)>) -> Self {
        Self {
            natural,
            tabbed,
            measured: Mutex::default(),
        }
    }

    fn measured(&self, tab: usize) -> Arc<Vec<usize>> {
        let mut measured = self.measured.lock().unwrap();
        if let Some(entry) = measured.iter().find(|entry| entry.tab == tab) {
            return entry.widths.clone();
        }
        let mut widths = self.natural.clone();
        for (column, text) in &self.tabbed {
            widths[*column] =
                widths[*column].max(wrap::display_column(text, text.chars().count(), tab));
        }
        let widths = Arc::new(widths);
        if measured.len() == 4 {
            measured.remove(0);
        }
        measured.push(MeasuredColumns {
            tab,
            widths: widths.clone(),
        });
        widths
    }
}

#[derive(Debug, Default)]
pub(crate) struct Tables {
    revision: u64,
    rows: Arc<[TableRow]>,
    cache: Mutex<Vec<CachedLayout>>,
}

#[derive(Debug)]
struct CachedLayout {
    key: (usize, usize, usize),
    layout: Arc<Layout>,
}

impl Clone for Tables {
    fn clone(&self) -> Self {
        Self {
            revision: self.revision,
            rows: self.rows.clone(),
            cache: Mutex::default(),
        }
    }
}

impl Tables {
    pub fn new(revision: u64, rows: Vec<TableRow>) -> Self {
        Self {
            revision,
            rows: rows.into(),
            cache: Mutex::default(),
        }
    }

    pub fn layout(
        &self,
        buffer: &Buffer,
        row: usize,
        width: usize,
        tab: usize,
    ) -> Option<Arc<Layout>> {
        if self.revision != buffer.revision() {
            return None;
        }
        let index = self
            .rows
            .binary_search_by_key(&row, |entry| entry.line)
            .ok()?;
        let mut cache = self.cache.lock().unwrap();
        if let Some(index) = cache
            .iter()
            .position(|entry| entry.key == (row, width, tab))
        {
            let entry = cache.remove(index);
            let layout = entry.layout.clone();
            cache.push(entry);
            return Some(layout);
        }
        let layout = Arc::new(Layout::new(buffer, &self.rows[index], width, tab));
        // Retain bounded geometry, as ordinary wrapping does. Very large rows
        // remain renderable but cannot evict the editor's memory budget.
        const BUDGET: usize = 2 * 1024 * 1024;
        if layout.cost() <= BUDGET {
            let mut cost = cache.iter().map(|entry| entry.layout.cost()).sum::<usize>();
            while cache.len() >= 16 || cost + layout.cost() > BUDGET {
                cost -= cache.remove(0).layout.cost();
            }
            cache.push(CachedLayout {
                key: (row, width, tab),
                layout: layout.clone(),
            });
        }
        Some(layout)
    }
}

#[derive(Debug)]
pub(crate) struct Cell {
    pub range: Range<usize>,
    pub x: usize,
    pub width: usize,
    pub spans: Vec<Segment>,
    // Character boundary positions measured once, including tab stops.
    positions: Vec<usize>,
}

#[derive(Debug)]
pub(crate) struct Layout {
    pub cells: Vec<Cell>,
    pub width: usize,
    pub height: usize,
    pub rule: bool,
    pub segments: Arc<[Segment]>,
}

/// Keep short columns natural; share a cap among longer columns. A minimum
/// of four cells (or the shorter natural width) leaves narrow tables readable.
/// If the minima cannot fit, the table overflows horizontally.
fn column_widths(natural: &[usize], width: usize) -> Vec<usize> {
    let available = width.saturating_sub(natural.len().saturating_sub(1) * 3);
    let minimum: Vec<_> = natural.iter().map(|n| (*n).clamp(1, 4)).collect();
    if minimum.iter().sum::<usize>() >= available {
        return minimum;
    }
    let mut low = 0;
    let mut high = natural.iter().copied().max().unwrap_or(1).max(1);
    while low < high {
        let mid = low + (high - low).div_ceil(2);
        let used: usize = natural
            .iter()
            .zip(&minimum)
            .map(|(n, m)| (*n).min(mid).max(*m))
            .sum();
        if used <= available {
            low = mid;
        } else {
            high = mid - 1;
        }
    }
    let mut widths: Vec<_> = natural
        .iter()
        .zip(&minimum)
        .map(|(n, m)| (*n).min(low).max(*m))
        .collect();
    let mut remaining = available.saturating_sub(widths.iter().sum());
    for (allocated, natural) in widths.iter_mut().zip(natural) {
        if remaining > 0 && *allocated < *natural {
            *allocated += 1;
            remaining -= 1;
        }
    }
    widths
}

impl Layout {
    fn new(buffer: &Buffer, row: &TableRow, width: usize, tab: usize) -> Self {
        let widths = column_widths(&row.widths.measured(tab), width);
        let mut x = 0;
        let mut cells = Vec::new();
        for (range, width) in row.cells.iter().zip(widths) {
            let text = buffer
                .text()
                .line(row.line)
                .slice(range.clone())
                .to_string();
            let mut position = 0;
            let mut positions = vec![0];
            for ch in text.chars() {
                position += if ch == '\t' {
                    tab.max(1) - position % tab.max(1)
                } else {
                    unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0)
                };
                positions.push(position);
            }
            let mut spans = wrap::segments(&text, width, tab);
            for span in &mut spans {
                span.start += range.start;
                span.end += range.start;
            }
            cells.push(Cell {
                range: range.clone(),
                x,
                width,
                spans,
                positions,
            });
            x += width + 3;
        }
        let height = if row.rule {
            1
        } else {
            cells.iter().map(|cell| cell.spans.len()).max().unwrap_or(1)
        };
        let count = height + usize::from(row.separator);
        let segments = (0..count)
            .map(|index| Segment {
                start: 0,
                end: buffer.line_len(row.line),
                start_cell: 0,
                end_cell: x.saturating_sub(3),
                table: Some(index),
            })
            .collect::<Vec<_>>()
            .into();
        Self {
            cells,
            width: x.saturating_sub(3),
            height,
            rule: row.rule,
            segments,
        }
    }

    fn cost(&self) -> usize {
        self.segments.len() * std::mem::size_of::<Segment>()
            + self
                .cells
                .iter()
                .map(|cell| {
                    std::mem::size_of::<Cell>()
                        + cell.positions.capacity() * std::mem::size_of::<usize>()
                        + cell.spans.capacity() * std::mem::size_of::<Segment>()
                })
                .sum::<usize>()
    }

    pub fn is_separator(&self, index: usize) -> bool {
        index >= self.height
    }

    /// Buffer column to visual row and horizontal cell. Original alignment
    /// spaces and rule characters remain valid offsets and map to the nearest
    /// visible boundary, without changing the buffer on resize.
    pub fn position(&self, column: usize) -> (usize, usize) {
        let Some(cell) = self
            .cells
            .iter()
            .rev()
            .find(|cell| column >= cell.range.start)
            .or(self.cells.first())
        else {
            return (0, 0);
        };
        if self.rule {
            let x = if column >= cell.range.end {
                cell.x + cell.width + column.saturating_sub(cell.range.end).min(2)
            } else {
                cell.x
                    + column
                        .saturating_sub(cell.range.start)
                        .min(cell.width.saturating_sub(1))
            };
            return (0, x.min(self.width.saturating_sub(1)));
        }
        let column = column.clamp(cell.range.start, cell.range.end);
        let index = cell
            .spans
            .partition_point(|span| span.end <= column)
            .min(cell.spans.len() - 1);
        let span = cell.spans[index];
        let within = cell.positions[column - cell.range.start].saturating_sub(span.start_cell);
        (index, cell.x + within.min(cell.width.saturating_sub(1)))
    }

    /// Hit testing distinguishes actual text from padding. Motion instead
    /// chooses the nearest populated fragment on this visual row, so an empty
    /// continuation in a short cell cannot trap vertical navigation.
    pub fn column(&self, index: usize, x: usize, exact: bool) -> Option<usize> {
        if self.is_separator(index) {
            return None;
        }
        if self.rule {
            if exact && x >= self.width {
                return None;
            }
            return self.rule_column(x.min(self.width.saturating_sub(1)));
        }
        let candidate = self
            .cells
            .iter()
            .filter_map(|cell| {
                let span = *cell.spans.get(index)?;
                let used = (span.end_cell - span.start_cell).min(cell.width);
                let distance = if x < cell.x {
                    cell.x - x
                } else {
                    x.saturating_sub(cell.x + used.saturating_sub(1))
                };
                Some((distance, cell, span, used))
            })
            .min_by_key(|candidate| candidate.0)?;
        let (_, cell, span, used) = candidate;
        if exact && (x < cell.x || x >= cell.x + used) {
            return None;
        }
        let desired = x.saturating_sub(cell.x).min(used.saturating_sub(1)) + span.start_cell;
        let positions =
            &cell.positions[span.start - cell.range.start..=span.end - cell.range.start];
        let character = positions
            .partition_point(|position| *position <= desired)
            .saturating_sub(1);
        Some(
            (span.start + character)
                .min(span.end.saturating_sub(usize::from(span.end > span.start))),
        )
    }

    fn rule_column(&self, x: usize) -> Option<usize> {
        let cell = self.cells.iter().rev().find(|cell| x >= cell.x)?;
        if x < cell.x + cell.width {
            Some(cell.range.start + (x - cell.x).min(cell.range.len().saturating_sub(1)))
        } else {
            Some(cell.range.end + (x - cell.x - cell.width).min(2))
        }
    }

    /// Scalars visible through a horizontal window. Synthetic padding and
    /// rules have no text offset. Work is bounded by the visible cells plus a
    /// small allowance for zero-width marks, even deep inside a large table.
    pub fn visible(
        &self,
        buffer: &Buffer,
        row: usize,
        index: usize,
        scroll: usize,
        width: usize,
    ) -> Vec<Atom> {
        let mut atoms = Vec::new();
        let end = scroll.saturating_add(width);
        let rule = self.rule || self.is_separator(index);
        for (column, cell) in self.cells.iter().enumerate() {
            if cell.x.saturating_sub(3) >= end {
                break;
            }
            if column > 0 {
                for (delta, ch) in if rule {
                    ['─', '┼', '─']
                } else {
                    [' ', '│', ' ']
                }
                .into_iter()
                .enumerate()
                {
                    let x = cell.x - 3 + delta;
                    if x >= scroll && x < end {
                        atoms.push(Atom {
                            x: x - scroll,
                            ch,
                            offset: self.rule.then(|| self.rule_column(x)).flatten(),
                            width: 1,
                        });
                    }
                }
            }
            if cell.x + cell.width <= scroll {
                continue;
            }
            let span = (!rule).then(|| cell.spans.get(index)).flatten().copied();
            let mut used = 0;
            if let Some(span) = span {
                let local = scroll.saturating_sub(cell.x) + span.start_cell;
                let positions =
                    &cell.positions[span.start - cell.range.start..=span.end - cell.range.start];
                let mut skip = positions
                    .partition_point(|p| *p < local)
                    .min(span.end - span.start);
                if skip > 0
                    && positions[skip] > local
                    && buffer.text().line(row).char(span.start + skip - 1) == '\t'
                {
                    skip -= 1;
                }
                used = positions[skip]
                    .saturating_sub(span.start_cell)
                    .min(cell.width);
                for (offset, ch) in buffer
                    .text()
                    .line(row)
                    .chars_at(span.start + skip)
                    .take((span.end - span.start - skip).min(width + 1024))
                    .enumerate()
                {
                    let offset = span.start + skip + offset;
                    let pos = offset - cell.range.start;
                    let x = cell.x + cell.positions[pos].saturating_sub(span.start_cell);
                    let size = (cell.positions[pos + 1] - cell.positions[pos])
                        .min(cell.width.saturating_sub(x.saturating_sub(cell.x)));
                    if x > end
                        || x > cell.x + cell.width
                        || size > 0 && (x == end || x == cell.x + cell.width)
                    {
                        break;
                    }
                    used = x - cell.x + size;
                    if ch == '\t' {
                        let visible_x = x.max(scroll);
                        let size = (x + size).min(end).saturating_sub(visible_x);
                        if size > 0 {
                            atoms.push(Atom {
                                x: visible_x - scroll,
                                ch,
                                offset: Some(offset),
                                width: size,
                            });
                        }
                    } else if x >= scroll && x + size <= end {
                        atoms.push(Atom {
                            x: x - scroll,
                            ch,
                            offset: Some(offset),
                            width: size,
                        });
                    }
                }
            }
            for x in (cell.x + used).max(scroll)..(cell.x + cell.width).min(end) {
                atoms.push(Atom {
                    x: x - scroll,
                    ch: if rule { '─' } else { ' ' },
                    offset: self.rule.then(|| self.rule_column(x)).flatten(),
                    width: 1,
                });
            }
        }
        atoms
    }
}

pub(crate) struct Atom {
    pub x: usize,
    pub ch: char,
    pub offset: Option<usize>,
    pub width: usize,
}

#[cfg(test)]
mod tests;
