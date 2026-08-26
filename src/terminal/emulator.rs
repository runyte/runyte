// SPDX-License-Identifier: MPL-2.0

//! What the parsed actions mean.
//!
//! The emulator owns the two screens a terminal has, the pen new characters
//! are written with, the modes a child has switched on, and the bytes that
//! have to be written back when the child asks a question. It is the only
//! place that turns an [`Action`](super::parser::Action) into a change on a
//! [`Grid`].

use super::{
    DefaultColors,
    grid::{Attributes, Color, Grid, Pen},
    parser::{Action, Parser, parameter, raw_parameter},
};
/// Modes a child switches on and off, and that key encoding has to honour.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Modes {
    /// DECCKM. Cursor keys send `SS3` finals instead of `CSI` ones.
    pub application_cursor_keys: bool,
    /// DECKPAM. Tracked so a future keypad encoding has it; nothing reads it
    /// yet, because Runyte's own input never distinguishes the keypad.
    pub application_keypad: bool,
    /// DECAWM, on by default as every real terminal has it.
    pub autowrap: bool,
    pub cursor_visible: bool,
    /// Bracketed paste. Pasted text is wrapped so a shell does not run it.
    pub bracketed_paste: bool,
    /// Any of the X10/normal/button/any mouse reporting modes.
    pub mouse_reporting: bool,
    /// SGR extended mouse coordinates (`DECSET 1006`), the one reporting
    /// encoding Runyte forwards.
    pub mouse_sgr: bool,
    /// Origin mode: row addressing is relative to the scrolling region.
    pub origin: bool,
    /// IRM: printing shifts the rest of the line right.
    pub insert: bool,
}

impl Default for Modes {
    fn default() -> Self {
        Self {
            application_cursor_keys: false,
            application_keypad: false,
            autowrap: true,
            cursor_visible: true,
            bracketed_paste: false,
            mouse_reporting: false,
            mouse_sgr: false,
            origin: false,
            insert: false,
        }
    }
}

/// The screen state of one child process.
#[derive(Debug)]
pub struct Emulator {
    parser: Parser,
    primary: Grid,
    alternate: Grid,
    alternate_active: bool,
    alternate_saved_cursor: bool,
    pen: Pen,
    pub modes: Modes,
    tab_stops: Vec<bool>,
    title: Option<String>,
    /// Most recent bounded OSC 7 payload. Validation against the local host
    /// and filesystem belongs to the owning session, not the emulator.
    directory_report: Option<Vec<u8>>,
    bell: bool,
    /// Bytes owed to the child in answer to a query it sent.
    replies: Vec<u8>,
    default_colors: DefaultColors,
}

impl Emulator {
    pub fn new(columns: usize, rows: usize) -> Self {
        let columns = columns.max(1);
        let rows = rows.max(1);
        Self {
            parser: Parser::new(),
            primary: Grid::new(columns, rows, true),
            alternate: Grid::new(columns, rows, false),
            alternate_active: false,
            alternate_saved_cursor: false,
            pen: Pen::default(),
            modes: Modes::default(),
            tab_stops: default_tab_stops(columns),
            title: None,
            directory_report: None,
            bell: false,
            replies: Vec::new(),
            default_colors: DefaultColors::default(),
        }
    }

    pub(super) fn set_default_colors(&mut self, colors: DefaultColors) {
        self.default_colors = colors;
    }

    pub fn grid(&self) -> &Grid {
        if self.alternate_active {
            &self.alternate
        } else {
            &self.primary
        }
    }

    pub(super) fn grid_mut(&mut self) -> &mut Grid {
        if self.alternate_active {
            &mut self.alternate
        } else {
            &mut self.primary
        }
    }

    pub fn alternate_screen(&self) -> bool {
        self.alternate_active
    }

    pub fn title(&self) -> Option<&str> {
        self.title.as_deref()
    }

    pub fn take_directory_report(&mut self) -> Option<Vec<u8>> {
        self.directory_report.take()
    }

    pub fn take_bell(&mut self) -> bool {
        std::mem::take(&mut self.bell)
    }

    /// Takes the bytes owed to the child, if any.
    pub fn take_replies(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.replies)
    }

    pub fn columns(&self) -> usize {
        self.grid().columns()
    }

    pub fn rows(&self) -> usize {
        self.grid().rows()
    }

    pub fn resize(&mut self, columns: usize, rows: usize) {
        let pen = self.pen;
        self.primary.resize(columns, rows, pen);
        self.alternate.resize(columns, rows, pen);
        self.tab_stops = default_tab_stops(columns.max(1));
    }

    /// The visible screen and the history behind it, as plain text.
    ///
    /// The alternate screen has no history by construction, so what comes back
    /// while `htop` is running is the screen and nothing more. That is the
    /// honest answer rather than a synthesised one.
    pub fn plain_text(&self) -> String {
        self.grid().plain_text()
    }

    pub fn feed(&mut self, bytes: &[u8]) {
        let mut actions = Vec::new();
        self.parser.advance(bytes, |action| actions.push(action));
        for action in actions {
            self.apply(action);
        }
    }

    fn apply(&mut self, action: Action) {
        match action {
            Action::Print(character) => self.print(character),
            Action::Execute(byte) => self.execute(byte),
            Action::Escape {
                intermediates,
                final_byte,
            } => self.escape(&intermediates, final_byte),
            Action::Csi {
                private,
                parameters,
                intermediates,
                final_byte,
            } => self.csi(private, &parameters, &intermediates, final_byte),
            Action::Osc(parameters) => self.osc(&parameters),
        }
    }

    fn print(&mut self, character: char) {
        let pen = self.pen;
        let autowrap = self.modes.autowrap;
        let insert = self.modes.insert;
        self.grid_mut()
            .write_with_insert(character, pen, autowrap, insert);
    }

    fn execute(&mut self, byte: u8) {
        let pen = self.pen;
        match byte {
            0x08 => self.grid_mut().backspace(),
            0x09 => self.tab_forward(1),
            0x0a..=0x0c => self.grid_mut().index(pen),
            0x0d => self.grid_mut().carriage_return(),
            // A bell is metadata only. Ringing the outer terminal would let a
            // child reach past its pane.
            0x07 => self.bell = true,
            _ => {}
        }
    }

    fn tab_forward(&mut self, count: usize) {
        let columns = self.grid().columns();
        let mut column = self.grid().cursor.column;
        for _ in 0..count.max(1) {
            let mut next = column + 1;
            while next < columns && !self.tab_stops.get(next).copied().unwrap_or(false) {
                next += 1;
            }
            column = next.min(columns - 1);
        }
        self.grid_mut().move_column(column);
    }

    fn tab_backward(&mut self, count: usize) {
        let mut column = self.grid().cursor.column;
        for _ in 0..count.max(1) {
            let mut previous = column;
            while previous > 0 {
                previous -= 1;
                if self.tab_stops.get(previous).copied().unwrap_or(false) {
                    break;
                }
            }
            column = previous;
        }
        self.grid_mut().move_column(column);
    }

    fn escape(&mut self, intermediates: &[u8], final_byte: u8) {
        let pen = self.pen;
        match (intermediates.first().copied(), final_byte) {
            (None, b'7') => {
                self.grid_mut().save_cursor(pen);
            }
            (None, b'8') => {
                if let Some(pen) = self.grid_mut().restore_cursor() {
                    self.pen = pen;
                }
            }
            (None, b'D') => self.grid_mut().index(pen),
            (None, b'E') => {
                self.grid_mut().index(pen);
                self.grid_mut().carriage_return();
            }
            (None, b'M') => self.grid_mut().reverse_index(pen),
            (None, b'H') => {
                let column = self.grid().cursor.column;
                if let Some(stop) = self.tab_stops.get_mut(column) {
                    *stop = true;
                }
            }
            (None, b'c') => self.reset(),
            (None, b'=') => self.modes.application_keypad = true,
            (None, b'>') => self.modes.application_keypad = false,
            // Character-set designation. Runyte decodes UTF-8 unconditionally,
            // so the only reason to notice these is not to print them.
            (Some(b'('..=b'+'), _) => {}
            (Some(b'#'), _) => {}
            _ => {}
        }
    }

    fn reset(&mut self) {
        let columns = self.grid().columns();
        let rows = self.grid().rows();
        self.primary = Grid::new(columns, rows, true);
        self.alternate = Grid::new(columns, rows, false);
        self.alternate_active = false;
        self.alternate_saved_cursor = false;
        self.pen = Pen::default();
        self.modes = Modes::default();
        self.tab_stops = default_tab_stops(columns);
        self.title = None;
    }

    fn csi(
        &mut self,
        private: Option<u8>,
        parameters: &[Vec<u16>],
        intermediates: &[u8],
        final_byte: u8,
    ) {
        let pen = self.pen;
        let first = usize::from(parameter(parameters, 0, 1));
        match (private, final_byte) {
            (None, b'@') => self.grid_mut().insert_characters(first, pen),
            (None, b'A') => self.grid_mut().move_up(first),
            (None, b'B' | b'e') => self.grid_mut().move_down(first),
            (None, b'C' | b'a') => self.grid_mut().move_right(first),
            (None, b'D') => self.grid_mut().move_left(first),
            (None, b'E') => {
                self.grid_mut().move_down(first);
                self.grid_mut().carriage_return();
            }
            (None, b'F') => {
                self.grid_mut().move_up(first);
                self.grid_mut().carriage_return();
            }
            (None, b'G' | b'`') => self.grid_mut().move_column(first.saturating_sub(1)),
            (None, b'H' | b'f') => {
                let row = first.saturating_sub(1);
                let column = usize::from(parameter(parameters, 1, 1)).saturating_sub(1);
                let row = self.absolute_row(row);
                self.grid_mut().move_to(row, column);
            }
            (None, b'I') => self.tab_forward(first),
            (None, b'J') => self
                .grid_mut()
                .erase_display(raw_parameter(parameters, 0), pen),
            (None, b'K') => self
                .grid_mut()
                .erase_line(raw_parameter(parameters, 0), pen),
            (None, b'L') => self.grid_mut().insert_lines(first, pen),
            (None, b'M') => self.grid_mut().delete_lines(first, pen),
            (None, b'P') => self.grid_mut().delete_characters(first, pen),
            (None, b'S') => self.grid_mut().scroll_up(first, pen),
            (None, b'T') => self.grid_mut().scroll_down(first, pen),
            (None, b'X') => self.grid_mut().erase_characters(first, pen),
            (None, b'Z') => self.tab_backward(first),
            (None, b'd') => {
                let row = self.absolute_row(first.saturating_sub(1));
                self.grid_mut().move_row(row);
            }
            (None, b'g') => self.clear_tab_stop(raw_parameter(parameters, 0)),
            (None, b'h') => self.set_ansi_modes(parameters, true),
            (None, b'l') => self.set_ansi_modes(parameters, false),
            (None, b'm') => self.select_graphic_rendition(parameters),
            (None, b'n') => self.device_status(raw_parameter(parameters, 0)),
            (None, b'r') if intermediates.is_empty() => {
                let rows = self.grid().rows();
                let top = usize::from(parameter(parameters, 0, 1)).saturating_sub(1);
                let bottom = usize::from(parameter(parameters, 1, rows as u16)).saturating_sub(1);
                self.grid_mut().set_scroll_region(top, bottom.min(rows - 1));
                let row = self.absolute_row(0);
                self.grid_mut().move_to(row, 0);
            }
            (None, b's') => {
                self.grid_mut().save_cursor(pen);
            }
            (None, b'u') => {
                if let Some(pen) = self.grid_mut().restore_cursor() {
                    self.pen = pen;
                }
            }
            (Some(b'?'), b'h') => self.set_private_modes(parameters, true),
            (Some(b'?'), b'l') => self.set_private_modes(parameters, false),
            // Primary device attributes: a VT102 with no options, which is
            // what a child needs to hear to stop asking.
            (None, b'c') => self.replies.extend_from_slice(b"\x1b[?6c"),
            // Window manipulation, cursor shape, and everything else a child
            // may try are deliberately ignored rather than guessed at.
            _ => {}
        }
    }

    /// Translates a row given by the child into a screen row, honouring the
    /// origin mode that makes addressing relative to the scrolling region.
    fn absolute_row(&self, row: usize) -> usize {
        if self.modes.origin {
            let (top, bottom) = self.grid().scroll_region();
            (top + row).min(bottom)
        } else {
            row
        }
    }

    fn clear_tab_stop(&mut self, mode: u16) {
        match mode {
            3 => self.tab_stops.iter_mut().for_each(|stop| *stop = false),
            _ => {
                let column = self.grid().cursor.column;
                if let Some(stop) = self.tab_stops.get_mut(column) {
                    *stop = false;
                }
            }
        }
    }

    fn device_status(&mut self, request: u16) {
        match request {
            5 => self.replies.extend_from_slice(b"\x1b[0n"),
            6 => {
                let cursor = self.grid().cursor;
                let (top, _) = self.grid().scroll_region();
                let row = if self.modes.origin {
                    cursor.row.saturating_sub(top)
                } else {
                    cursor.row
                };
                let reply = format!("\x1b[{};{}R", row + 1, cursor.column + 1);
                self.replies.extend_from_slice(reply.as_bytes());
            }
            _ => {}
        }
    }

    fn set_ansi_modes(&mut self, parameters: &[Vec<u16>], enabled: bool) {
        for index in 0..parameters.len().max(1) {
            if raw_parameter(parameters, index) == 4 {
                self.modes.insert = enabled;
            }
        }
    }

    fn set_private_modes(&mut self, parameters: &[Vec<u16>], enabled: bool) {
        for index in 0..parameters.len().max(1) {
            match raw_parameter(parameters, index) {
                1 => self.modes.application_cursor_keys = enabled,
                6 => {
                    self.modes.origin = enabled;
                    let row = self.absolute_row(0);
                    self.grid_mut().move_to(row, 0);
                }
                7 => self.modes.autowrap = enabled,
                25 => self.modes.cursor_visible = enabled,
                1000 | 1002 | 1003 => self.modes.mouse_reporting = enabled,
                1006 => self.modes.mouse_sgr = enabled,
                47 => self.set_alternate_screen(enabled, false, false, false),
                1047 => self.set_alternate_screen(enabled, false, true, false),
                1049 => self.set_alternate_screen(enabled, true, false, true),
                2004 => self.modes.bracketed_paste = enabled,
                _ => {}
            }
        }
    }

    fn set_alternate_screen(
        &mut self,
        enabled: bool,
        clear_on_enter: bool,
        clear_on_exit: bool,
        save_cursor: bool,
    ) {
        if enabled == self.alternate_active {
            return;
        }
        let pen = self.pen;
        if enabled {
            if save_cursor {
                self.primary.save_cursor(pen);
                self.alternate_saved_cursor = true;
            }
            if clear_on_enter {
                self.alternate.move_to(0, 0);
                self.alternate.erase_display(2, Pen::default());
            }
            self.alternate_active = true;
        } else {
            if clear_on_exit {
                self.alternate.erase_display(2, Pen::default());
            }
            self.alternate_active = false;
            if self.alternate_saved_cursor {
                if let Some(pen) = self.primary.restore_cursor() {
                    self.pen = pen;
                }
                self.alternate_saved_cursor = false;
            }
        }
    }

    fn select_graphic_rendition(&mut self, parameters: &[Vec<u16>]) {
        if parameters.is_empty() {
            self.pen = Pen::default();
            return;
        }
        let mut index = 0;
        while index < parameters.len() {
            let values = &parameters[index];
            match values.first().copied().unwrap_or(0) {
                0 => self.pen = Pen::default(),
                1 => self.pen.attributes.insert(Attributes::BOLD),
                2 => self.pen.attributes.insert(Attributes::DIM),
                3 => self.pen.attributes.insert(Attributes::ITALIC),
                4 => self.pen.attributes.insert(Attributes::UNDERLINE),
                5 | 6 => self.pen.attributes.insert(Attributes::BLINK),
                7 => self.pen.attributes.insert(Attributes::REVERSE),
                8 => self.pen.attributes.insert(Attributes::HIDDEN),
                9 => self.pen.attributes.insert(Attributes::STRIKETHROUGH),
                21 | 22 => self.pen.attributes.remove(Attributes::from_bits(
                    Attributes::BOLD.bits() | Attributes::DIM.bits(),
                )),
                23 => self.pen.attributes.remove(Attributes::ITALIC),
                24 => self.pen.attributes.remove(Attributes::UNDERLINE),
                25 => self.pen.attributes.remove(Attributes::BLINK),
                27 => self.pen.attributes.remove(Attributes::REVERSE),
                28 => self.pen.attributes.remove(Attributes::HIDDEN),
                29 => self.pen.attributes.remove(Attributes::STRIKETHROUGH),
                value @ 30..=37 => self.pen.foreground = Color::Indexed((value - 30) as u8),
                38 => {
                    let (color, consumed) = extended_color(parameters, index, values);
                    if let Some(color) = color {
                        self.pen.foreground = color;
                    }
                    index += consumed;
                }
                39 => self.pen.foreground = Color::Default,
                value @ 40..=47 => self.pen.background = Color::Indexed((value - 40) as u8),
                48 => {
                    let (color, consumed) = extended_color(parameters, index, values);
                    if let Some(color) = color {
                        self.pen.background = color;
                    }
                    index += consumed;
                }
                49 => self.pen.background = Color::Default,
                value @ 90..=97 => self.pen.foreground = Color::Indexed((value - 90 + 8) as u8),
                value @ 100..=107 => self.pen.background = Color::Indexed((value - 100 + 8) as u8),
                _ => {}
            }
            index += 1;
        }
    }

    fn osc(&mut self, parameters: &[Vec<u8>]) {
        let Some(kind) = parameters.first() else {
            return;
        };
        match kind.as_slice() {
            // Icon name and window title. Both become the pane's title,
            // because a pane has one name and the title is the useful one.
            b"0" | b"1" | b"2" => {
                let title = parameters.get(1).map(|value| sanitize(value));
                self.title = title.filter(|title| !title.is_empty());
            }
            b"7" => {
                self.directory_report = parameters
                    .get(1)
                    .filter(|value| value.len() <= 4096)
                    .cloned();
            }
            b"10" if is_single_query(parameters) => {
                self.reply_default_color(10, self.default_colors.foreground());
            }
            b"11" if is_single_query(parameters) => {
                self.reply_default_color(11, self.default_colors.background());
            }
            // Everything else — hyperlinks, clipboard, palette queries, and
            // attempts to set a default colour — is ignored. OSC 10/11 expose
            // only the two colours the child is already being rendered on;
            // OSC 52 must not write the person's clipboard unasked.
            _ => {}
        }
    }

    fn reply_default_color(&mut self, kind: u8, color: Option<(u8, u8, u8)>) {
        let Some((red, green, blue)) = color else {
            return;
        };
        let reply = format!(
            "\x1b]{kind};rgb:{red:02x}{red:02x}/{green:02x}{green:02x}/{blue:02x}{blue:02x}\x1b\\"
        );
        self.replies.extend_from_slice(reply.as_bytes());
    }
}

fn is_single_query(parameters: &[Vec<u8>]) -> bool {
    matches!(parameters, [_, query] if query.as_slice() == b"?")
}

/// Reads a `38`/`48` extended colour, reporting how many further parameters it
/// consumed when written with semicolons rather than colons.
fn extended_color(parameters: &[Vec<u16>], index: usize, values: &[u16]) -> (Option<Color>, usize) {
    // Colon form: everything is inside this one parameter.
    if values.len() > 1 {
        return match values {
            [_, 5, value] => (u8::try_from(*value).ok().map(Color::Indexed), 0),
            [_, 2, ..] => {
                // `38:2::r:g:b` carries an empty colour-space field, and
                // `38:2:r:g:b` omits it. Take the last three either way.
                let channels = match values {
                    [_, 2, red, green, blue] => Some([*red, *green, *blue]),
                    [_, 2, 0, red, green, blue] => Some([*red, *green, *blue]),
                    _ => None,
                };
                let color = channels.and_then(|[red, green, blue]| {
                    Some(Color::Rgb(
                        u8::try_from(red).ok()?,
                        u8::try_from(green).ok()?,
                        u8::try_from(blue).ok()?,
                    ))
                });
                (color, 0)
            }
            _ => (None, 0),
        };
    }
    // Semicolon form: the following parameters belong to this colour.
    match parameters.get(index + 1).map(Vec::as_slice) {
        Some([5]) => (
            parameters
                .get(index + 2)
                .and_then(|values| match values.as_slice() {
                    [value] => u8::try_from(*value).ok(),
                    _ => None,
                })
                .map(Color::Indexed),
            2,
        ),
        Some([2]) => {
            let channels = (index + 2..=index + 4)
                .map(|channel| {
                    parameters
                        .get(channel)
                        .and_then(|values| match values.as_slice() {
                            [value] => u8::try_from(*value).ok(),
                            _ => None,
                        })
                })
                .collect::<Option<Vec<_>>>();
            let color = channels.and_then(|channels| match channels.as_slice() {
                [red, green, blue] => Some(Color::Rgb(*red, *green, *blue)),
                _ => None,
            });
            (color, 4)
        }
        _ => (None, 1),
    }
}

fn default_tab_stops(columns: usize) -> Vec<bool> {
    (0..columns).map(|column| column % 8 == 0).collect()
}

/// Keeps a child's title printable and one line long.
fn sanitize(value: &[u8]) -> String {
    String::from_utf8_lossy(value)
        .chars()
        .filter(|character| !character.is_control())
        .take(128)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(emulator: &Emulator, row: usize) -> String {
        let line = emulator.grid().line(row).unwrap();
        let end = line
            .iter()
            .rposition(|cell| cell.width != 0 && cell.character != ' ')
            .map_or(0, |index| index + 1);
        line[..end]
            .iter()
            .filter(|cell| cell.width != 0)
            .flat_map(|cell| cell.text().chars().collect::<Vec<_>>())
            .collect()
    }

    #[test]
    fn cursor_addressing_is_one_based_and_clamped() {
        let mut emulator = Emulator::new(10, 4);
        emulator.feed(b"\x1b[2;3Hxy");
        assert_eq!(row(&emulator, 1), "  xy");
        emulator.feed(b"\x1b[99;99Hz");
        assert_eq!(emulator.grid().cursor.row, 3);
    }

    #[test]
    fn sgr_reads_both_the_colon_and_the_semicolon_true_colour_forms() {
        let mut emulator = Emulator::new(10, 2);
        emulator.feed(b"\x1b[38;2;10;20;30ma");
        assert_eq!(
            emulator.grid().line(0).unwrap()[0].foreground,
            Color::Rgb(10, 20, 30)
        );
        emulator.feed(b"\x1b[38:2::40:50:60mb");
        assert_eq!(
            emulator.grid().line(0).unwrap()[1].foreground,
            Color::Rgb(40, 50, 60)
        );
        emulator.feed(b"\x1b[38;5;200mc");
        assert_eq!(
            emulator.grid().line(0).unwrap()[2].foreground,
            Color::Indexed(200)
        );
    }

    #[test]
    fn invalid_extended_colours_do_not_wrap_or_fabricate_channels() {
        let mut emulator = Emulator::new(12, 2);
        emulator.feed(b"\x1b[31m\x1b[38;5;300ma\x1b[38;2;1;2mb\x1b[38:2:1:2:999mc\x1b[38:5:999md");

        for cell in &emulator.grid().line(0).unwrap()[..4] {
            assert_eq!(cell.foreground, Color::Indexed(1));
        }
    }

    #[test]
    fn bright_colours_land_in_the_upper_half_of_the_palette() {
        let mut emulator = Emulator::new(4, 1);
        emulator.feed(b"\x1b[92ma");
        assert_eq!(
            emulator.grid().line(0).unwrap()[0].foreground,
            Color::Indexed(10)
        );
    }

    #[test]
    fn the_alternate_screen_is_separate_and_keeps_no_history() {
        let mut emulator = Emulator::new(10, 2);
        emulator.feed(b"kept\r\n");
        emulator.feed(b"\x1b[?1049h");
        assert!(emulator.alternate_screen());
        emulator.feed(b"live");
        assert_eq!(row(&emulator, 0), "live");
        assert_eq!(emulator.grid().scrollback_len(), 0);
        emulator.feed(b"\x1b[?1049l");
        assert!(!emulator.alternate_screen());
        assert_eq!(row(&emulator, 0), "kept");
    }

    #[test]
    fn alternate_screen_modes_keep_their_distinct_clear_and_save_semantics() {
        let mut emulator = Emulator::new(8, 2);
        emulator.feed(b"\x1b[31mab\x1b[?47hkept\x1b[?47l\x1b[?47h");
        assert_eq!(row(&emulator, 0), "kept");
        emulator.feed(b"\x1b[?47l\x1b[?1047h");
        assert_eq!(row(&emulator, 0), "kept");
        emulator.feed(b"\x1b[?1047l\x1b[?47h");
        assert_eq!(row(&emulator, 0), "");

        emulator.feed(b"alt\x1b[?47l\x1b[?1049h\x1b[32m\x1b7moved\x1b[?1049lX");
        assert!(!emulator.alternate_screen());
        assert_eq!(row(&emulator, 0), "abX");
        assert_eq!(
            emulator.grid().line(0).unwrap()[2].foreground,
            Color::Indexed(1),
            "1049 restores the primary pen even after an alternate-screen save"
        );
    }

    #[test]
    fn insert_mode_shifts_by_the_printed_glyph_width() {
        let mut emulator = Emulator::new(6, 1);
        emulator.feed(b"abcd\x1b[1;2H\x1b[4h");
        emulator.feed("界".as_bytes());

        assert_eq!(row(&emulator, 0), "a界bcd");
        assert_eq!(emulator.grid().line(0).unwrap()[1].width, 2);
        assert_eq!(emulator.grid().line(0).unwrap()[2].width, 0);
    }

    #[test]
    fn insert_mode_resolves_delayed_wrap_before_shifting() {
        let mut emulator = Emulator::new(4, 2);
        emulator.feed(b"\x1b[2;1Hxyz\x1b[1;1H1234\x1b[4ha");

        assert_eq!(row(&emulator, 0), "1234");
        assert_eq!(row(&emulator, 1), "axyz");
    }

    #[test]
    fn insert_mode_resolves_wide_overflow_before_shifting() {
        let mut emulator = Emulator::new(6, 2);
        emulator.feed(b"\x1b[2;1Hxyz\x1b[1;6H\x1b[4h");
        emulator.feed("界".as_bytes());

        assert_eq!(row(&emulator, 1), "界xyz");
        assert_eq!(emulator.grid().line(1).unwrap()[0].width, 2);
        assert_eq!(emulator.grid().line(1).unwrap()[1].width, 0);
    }

    #[test]
    fn a_cursor_position_report_answers_the_child() {
        let mut emulator = Emulator::new(10, 4);
        emulator.feed(b"\x1b[3;5H\x1b[6n");
        assert_eq!(emulator.take_replies(), b"\x1b[3;5R".to_vec());
        assert!(emulator.take_replies().is_empty());
    }

    #[test]
    fn a_title_sequence_names_the_terminal_and_control_bytes_are_dropped() {
        let mut emulator = Emulator::new(10, 2);
        emulator.feed(b"\x1b]0;bash \x07");
        assert_eq!(emulator.title(), Some("bash "));
    }

    #[test]
    fn default_colour_queries_answer_with_the_current_theme_colours() {
        let mut emulator = Emulator::new(10, 2);
        emulator.set_default_colors(DefaultColors::new(
            Some((0x24, 0x29, 0x2f)),
            Some((0xfb, 0xfb, 0xfa)),
        ));

        emulator.feed(b"\x1b]10;?\x07\x1b]11;?\x1b\\");

        assert_eq!(
            emulator.take_replies(),
            b"\x1b]10;rgb:2424/2929/2f2f\x1b\\\x1b]11;rgb:fbfb/fbfb/fafa\x1b\\"
        );
        assert!(emulator.take_replies().is_empty());
    }

    #[test]
    fn default_colour_queries_do_not_expand_into_palette_or_clipboard_access() {
        let mut emulator = Emulator::new(10, 2);
        emulator.set_default_colors(DefaultColors::new(Some((1, 2, 3)), None));

        emulator.feed(
            b"\x1b]10;rgb:ffff/ffff/ffff\x07\x1b]10;?;?\x07\x1b]11;?\x07\x1b]4;0;?\x07\x1b]52;c;payload\x07",
        );

        assert!(emulator.take_replies().is_empty());
        emulator.feed(b"\x1b]10;?\x07");
        assert_eq!(emulator.take_replies(), b"\x1b]10;rgb:0101/0202/0303\x1b\\");
    }

    #[test]
    fn a_scroll_region_confines_line_insertion() {
        let mut emulator = Emulator::new(6, 4);
        emulator.feed(b"\x1b[1;1Hone\x1b[2;1Htwo\x1b[3;1Hsix\x1b[4;1Hend");
        emulator.feed(b"\x1b[1;3r\x1b[1;1H\x1b[L");
        assert_eq!(row(&emulator, 0), "");
        assert_eq!(row(&emulator, 1), "one");
        assert_eq!(row(&emulator, 2), "two");
        assert_eq!(row(&emulator, 3), "end");
    }

    #[test]
    fn application_cursor_keys_and_bracketed_paste_are_tracked() {
        let mut emulator = Emulator::new(4, 1);
        assert!(!emulator.modes.application_cursor_keys);
        emulator.feed(b"\x1b[?1h\x1b[?2004h");
        assert!(emulator.modes.application_cursor_keys);
        assert!(emulator.modes.bracketed_paste);
        emulator.feed(b"\x1b[?1l\x1b[?2004l");
        assert!(!emulator.modes.application_cursor_keys);
        assert!(!emulator.modes.bracketed_paste);
    }

    #[test]
    fn a_tab_lands_on_the_next_eight_column_stop() {
        let mut emulator = Emulator::new(24, 1);
        emulator.feed(b"ab\tc");
        assert_eq!(row(&emulator, 0), "ab      c");
    }
}
