// SPDX-License-Identifier: MPL-2.0

//! Complete immutable grids whose unchanged rows are shared across frames.
//! Ratatui's cell diff writes through copy-on-write rows, never a full-grid copy.
use ratatui::{
    backend::{Backend, ClearType, WindowSize},
    buffer::{Buffer, Cell},
    layout::{Position, Rect, Size},
};
use std::{
    convert::Infallible,
    ops::{Index, IndexMut},
    sync::Arc,
};

#[derive(Clone)]
pub(super) struct Grid {
    pub area: Rect,
    pub rows: Vec<Arc<[Cell]>>,
}
impl Grid {
    fn empty(area: Rect) -> Self {
        let row: Arc<[Cell]> = vec![Cell::default(); area.width as usize].into();
        Self {
            area,
            rows: vec![row; area.height as usize],
        }
    }
}
impl From<Buffer> for Grid {
    fn from(buffer: Buffer) -> Self {
        let mut grid = Self::empty(buffer.area);
        if buffer.area.width > 0 {
            grid.rows = buffer
                .content
                .chunks(buffer.area.width as usize)
                .map(|row| Arc::from(row.to_vec()))
                .collect();
        }
        grid
    }
}
impl Index<(u16, u16)> for Grid {
    type Output = Cell;
    fn index(&self, (x, y): (u16, u16)) -> &Cell {
        &self.rows[(y - self.area.y) as usize][(x - self.area.x) as usize]
    }
}
impl IndexMut<(u16, u16)> for Grid {
    fn index_mut(&mut self, (x, y): (u16, u16)) -> &mut Cell {
        &mut Arc::make_mut(&mut self.rows[(y - self.area.y) as usize])[(x - self.area.x) as usize]
    }
}

/// Fullscreen-only backend. Each diff clones at most the rows it changes;
/// publishing clones row references, so skipped frames cannot lose updates.
pub(crate) struct GridBackend {
    grid: Grid,
    cursor: Position,
    visible: bool,
}
impl GridBackend {
    pub(super) fn new(width: u16, height: u16) -> Self {
        Self {
            grid: Grid::empty(Rect::new(0, 0, width, height)),
            cursor: Position::ORIGIN,
            visible: false,
        }
    }
    pub(super) fn resize(&mut self, width: u16, height: u16) {
        let area = Rect::new(0, 0, width, height);
        if self.grid.area != area {
            self.grid = Grid::empty(area);
        }
    }
    pub(super) fn snapshot(&self) -> Grid {
        self.grid.clone()
    }
    pub(super) fn cursor_visible(&self) -> bool {
        self.visible
    }
    pub(super) fn cursor_position(&self) -> Position {
        self.cursor
    }
}
impl Backend for GridBackend {
    type Error = Infallible;
    fn draw<'a, I>(&mut self, content: I) -> Result<(), Self::Error>
    where
        I: Iterator<Item = (u16, u16, &'a Cell)>,
    {
        for (x, y, cell) in content {
            self.grid[(x, y)].clone_from(cell);
        }
        Ok(())
    }
    fn hide_cursor(&mut self) -> Result<(), Self::Error> {
        self.visible = false;
        Ok(())
    }
    fn show_cursor(&mut self) -> Result<(), Self::Error> {
        self.visible = true;
        Ok(())
    }
    fn get_cursor_position(&mut self) -> Result<Position, Self::Error> {
        Ok(self.cursor)
    }
    fn set_cursor_position<P: Into<Position>>(&mut self, position: P) -> Result<(), Self::Error> {
        self.cursor = position.into();
        Ok(())
    }
    fn clear(&mut self) -> Result<(), Self::Error> {
        self.grid = Grid::empty(self.grid.area);
        Ok(())
    }
    fn clear_region(&mut self, region: ClearType) -> Result<(), Self::Error> {
        if region == ClearType::All {
            return self.clear();
        }
        for y in 0..self.grid.area.height {
            for x in 0..self.grid.area.width {
                let reset = match region {
                    ClearType::All => true,
                    ClearType::AfterCursor => (y, x) >= (self.cursor.y, self.cursor.x),
                    ClearType::BeforeCursor => (y, x) <= (self.cursor.y, self.cursor.x),
                    ClearType::CurrentLine => y == self.cursor.y,
                    ClearType::UntilNewLine => y == self.cursor.y && x >= self.cursor.x,
                };
                if reset {
                    self.grid[(x, y)].reset();
                }
            }
        }
        Ok(())
    }
    fn size(&self) -> Result<Size, Self::Error> {
        Ok(self.grid.area.as_size())
    }
    fn window_size(&mut self) -> Result<WindowSize, Self::Error> {
        Ok(WindowSize {
            columns_rows: self.grid.area.as_size(),
            pixels: Size::default(),
        })
    }
    fn flush(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }
}

#[cfg(test)]
#[path = "tests/grid.rs"]
mod tests;
