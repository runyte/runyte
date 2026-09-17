// SPDX-License-Identifier: MPL-2.0

//! Bounded, owned readings of live emulator output. These are internal editor
//! values, not a wire protocol or an authorization boundary. Reading never
//! changes native review, input, attention, or viewport state.

use super::{Cell, TerminalId, TerminalLineId, TerminalSession};

pub const MAX_ROWS: usize = 1_000;
pub const MAX_BYTES: usize = 256 * 1024;
pub const MAX_CELLS: usize = 256 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Region {
    /// First rows of the active live screen, excluding its scrollback.
    Screen,
    /// Newest rows of the active grid, including retained primary history.
    Tail,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Limits {
    pub rows: usize,
    /// UTF-8 bytes across returned row strings, excluding framing/metadata.
    pub bytes: usize,
    /// Includes blank and wide-continuation cells, even if no text is emitted.
    pub cells: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            rows: 200,
            bytes: 64 * 1024,
            cells: 64 * 1024,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReadError {
    InvalidLimits,
    Stale { actual_revision: u64 },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Row {
    /// Zero-based position in retained history followed by the live screen.
    pub index: usize,
    pub id: TerminalLineId,
    /// A prefix of the decoded row, with plain trailing blanks omitted.
    /// A cell's base and combining marks are never separated by a byte limit.
    pub text: String,
    /// Cells represented by this prefix, including omitted trailing blanks.
    pub columns: usize,
    pub clipped: bool,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Truncation {
    pub rows: bool,
    pub bytes: bool,
    pub cells: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Snapshot {
    pub terminal: TerminalId,
    pub revision: u64,
    pub region: Region,
    pub alternate_screen: bool,
    pub live: bool,
    pub columns: usize,
    pub screen_rows: usize,
    pub history_rows: usize,
    /// Previously retired rows no longer retained in this grid. Resetting
    /// the emulator starts a new grid; alternate screens have no history.
    pub lost_history_rows: u64,
    /// Number of rows available in the requested region before response limits.
    pub available_rows: usize,
    /// Ascending source order, even though a tail's newest rows get first
    /// use of the byte/cell budget. At most one returned row is clipped.
    pub rows: Vec<Row>,
    pub truncation: Truncation,
    pub visited_cells: usize,
}

impl TerminalSession {
    /// Conservative live-output identity: output chunks (including controls),
    /// resize, exit and workspace history eviction advance it. UI review and
    /// scrolling do not. Scope this value to the owning terminal and host.
    pub fn read_revision(&self) -> u64 {
        self.read_revision
    }

    /// Captures only the requested live rows. The result owns its text and
    /// stays immutable while the child continues writing. Host-side snapshot
    /// retention must charge these allocations separately before keeping them.
    pub fn read_output(
        &self,
        region: Region,
        limits: Limits,
        expected_revision: Option<u64>,
    ) -> Result<Snapshot, ReadError> {
        if !(1..=MAX_ROWS).contains(&limits.rows)
            || !(1..=MAX_BYTES).contains(&limits.bytes)
            || !(1..=MAX_CELLS).contains(&limits.cells)
        {
            return Err(ReadError::InvalidLimits);
        }
        if expected_revision.is_some_and(|expected| expected != self.read_revision) {
            return Err(ReadError::Stale {
                actual_revision: self.read_revision,
            });
        }
        let grid = self.emulator.grid();
        let history = grid.scrollback_len();
        let available = match region {
            Region::Screen => grid.rows(),
            Region::Tail => grid.plain_line_count(),
        };
        let count = available.min(limits.rows);
        let mut result = Snapshot {
            terminal: self.id,
            revision: self.read_revision,
            region,
            alternate_screen: self.alternate_screen(),
            live: self.live(),
            columns: grid.columns(),
            screen_rows: grid.rows(),
            history_rows: history,
            lost_history_rows: grid.retired().saturating_sub(history as u64),
            available_rows: available,
            rows: Vec::with_capacity(count),
            truncation: Truncation {
                rows: count < available,
                ..Truncation::default()
            },
            visited_cells: 0,
        };
        let mut bytes_left = limits.bytes;
        let mut cells_left = limits.cells;
        for offset in 0..count {
            if cells_left == 0 {
                result.truncation.cells = true;
                break;
            }
            let index = match region {
                Region::Screen => history + offset,
                Region::Tail => grid.plain_line_count() - 1 - offset,
            };
            let cells = if index < history {
                grid.scrollback_line(index)
            } else {
                grid.line(index - history)
            }
            .expect("requested row is retained");
            let (text, columns) = decode_row(
                cells,
                &mut bytes_left,
                &mut cells_left,
                &mut result.truncation,
            );
            let clipped = columns < cells.len();
            result.rows.push(Row {
                index,
                id: grid
                    .retained_line_id(index)
                    .expect("retained row has an ID"),
                text,
                columns,
                clipped,
            });
            if clipped {
                break;
            }
        }
        if region == Region::Tail {
            result.rows.reverse();
        }
        result.visited_cells = limits.cells - cells_left;
        Ok(result)
    }
}

fn decode_row(
    cells: &[Cell],
    bytes_left: &mut usize,
    cells_left: &mut usize,
    truncation: &mut Truncation,
) -> (String, usize) {
    let mut text = String::new();
    let mut blanks = 0;
    for (column, cell) in cells.iter().enumerate() {
        if *cells_left == 0 {
            truncation.cells = true;
            return (text, column);
        }
        if cell.width == 2 && *cells_left < 2 {
            *cells_left -= 1;
            truncation.cells = true;
            return (text, column);
        }
        *cells_left -= 1;
        if cell.width == 0 {
            continue;
        }
        if cell.character == ' ' && cell.combining_len == 0 {
            blanks += 1;
            continue;
        }
        let combining = &cell.combining[..usize::from(cell.combining_len)];
        let needed = blanks
            + cell.character.len_utf8()
            + combining.iter().map(|mark| mark.len_utf8()).sum::<usize>();
        if needed > *bytes_left {
            truncation.bytes = true;
            return (text, column);
        }
        text.extend(std::iter::repeat_n(' ', blanks));
        blanks = 0;
        text.push(cell.character);
        text.extend(combining);
        *bytes_left -= needed;
    }
    (text, cells.len())
}

#[cfg(test)]
#[path = "tests/read.rs"]
mod tests;
