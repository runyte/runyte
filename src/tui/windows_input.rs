// SPDX-License-Identifier: MPL-2.0

//! Native console input. VT input is essential: ConPTY otherwise consumes
//! bracketed-paste delimiters before ReadConsoleInput can observe them.
//! Crossterm's Windows decoder discards VK=0 control units, so it cannot read
//! this mode. Keep native UTF-16 records until paste framing is complete.

use crossterm::event::{
    Event, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use std::{
    io::{self, Write},
    ptr, thread,
    time::{Duration, Instant},
};
use tokio::sync::mpsc;
use windows_sys::Win32::{
    Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE, WAIT_OBJECT_0},
    System::{
        Console::*,
        Threading::{CreateEventW, SetEvent, WaitForMultipleObjects},
    },
};

const ESCAPE_DELAY: Duration = Duration::from_millis(30);
const MAX_PASTE_UNITS: usize = 1024 * 1024 + 1;

/// Keep native key identity when ConPTY serializes records into VT input.
/// The guard also rolls back a partial write during initialization.
pub struct KeyboardMode;
impl KeyboardMode {
    pub fn enable() -> io::Result<Self> {
        let guard = Self;
        let mut output = io::stdout().lock();
        output.write_all(b"\x1b[?9001h")?;
        output.flush()?;
        Ok(guard)
    }
}
impl Drop for KeyboardMode {
    fn drop(&mut self) {
        let mut output = io::stdout().lock();
        let _ = output.write_all(b"\x1b[?9001l");
        let _ = output.flush();
    }
}

/// Restores the exact incoming console mode, including VT and Quick Edit bits.
pub struct ConsoleMode {
    handle: HANDLE,
    original: u32,
}
impl ConsoleMode {
    pub fn capture() -> io::Result<Self> {
        let handle = unsafe { GetStdHandle(STD_INPUT_HANDLE) };
        let mut original = 0;
        if handle == INVALID_HANDLE_VALUE || unsafe { GetConsoleMode(handle, &mut original) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Self { handle, original })
    }
    pub fn enable_vt(&self) -> io::Result<()> {
        let mut mode = 0;
        if unsafe { GetConsoleMode(self.handle, &mut mode) } == 0
            || unsafe { SetConsoleMode(self.handle, mode | ENABLE_VIRTUAL_TERMINAL_INPUT) } == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
}
impl Drop for ConsoleMode {
    fn drop(&mut self) {
        unsafe {
            SetConsoleMode(self.handle, self.original);
        }
    }
}

pub struct EventStream {
    receiver: mpsc::Receiver<io::Result<Event>>,
    stop: HANDLE,
    reader: Option<thread::JoinHandle<()>>,
}
impl EventStream {
    /// Start after TerminalGuard has enabled native keyboard reporting on the
    /// supported Windows console host. An encoded Escape has a native VK and
    /// does not need the ambiguous legacy single-byte Escape timeout.
    pub fn new() -> io::Result<Self> {
        let input = unsafe { GetStdHandle(STD_INPUT_HANDLE) };
        let stop = unsafe { CreateEventW(ptr::null(), 1, 0, ptr::null()) };
        if stop.is_null() {
            return Err(io::Error::last_os_error());
        }
        let (sender, receiver) = mpsc::channel(64);
        // Handles stay owned by this stream until its reader has joined.
        let input_value = input as usize;
        let stop_value = stop as usize;
        let reader = match thread::Builder::new()
            .name("console-input".into())
            .spawn(move || {
                if let Err(error) =
                    read_events(input_value as HANDLE, stop_value as HANDLE, &sender)
                {
                    let _ = sender.blocking_send(Err(error));
                }
            }) {
            Ok(reader) => reader,
            Err(error) => {
                unsafe {
                    CloseHandle(stop);
                }
                return Err(error);
            }
        };
        Ok(Self {
            receiver,
            stop,
            reader: Some(reader),
        })
    }
    pub async fn next(&mut self) -> Option<io::Result<Event>> {
        self.receiver.recv().await
    }
}
impl Drop for EventStream {
    fn drop(&mut self) {
        self.receiver.close();
        unsafe {
            SetEvent(self.stop);
        }
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
        unsafe {
            CloseHandle(self.stop);
        }
    }
}

fn read_events(
    input: HANDLE,
    stop: HANDLE,
    sender: &mpsc::Sender<io::Result<Event>>,
) -> io::Result<()> {
    // TerminalGuard negotiates native reporting before this reader starts.
    // Even the first fragmented wire frame must not use a legacy ESC timeout.
    let mut decoder = Decoder::for_console();
    loop {
        let wait = if decoder.legacy_escape_pending() {
            ESCAPE_DELAY.as_millis() as u32
        } else {
            u32::MAX
        };
        match unsafe { WaitForMultipleObjects(2, [stop, input].as_ptr(), 0, wait) } {
            WAIT_OBJECT_0 => return Ok(()),
            value if value == WAIT_OBJECT_0 + 1 => {
                let mut record: INPUT_RECORD = unsafe { std::mem::zeroed() };
                let mut count = 0;
                if unsafe { ReadConsoleInputW(input, &mut record, 1, &mut count) } == 0 {
                    return Err(io::Error::last_os_error());
                }
                if count == 0 {
                    return Ok(());
                }
                for event in decoder.record(&record) {
                    if sender.blocking_send(Ok(event)).is_err() {
                        return Ok(());
                    }
                }
            }
            windows_sys::Win32::Foundation::WAIT_TIMEOUT => {
                if let Some(event) = decoder.escape_timeout(Instant::now())
                    && sender.blocking_send(Ok(event)).is_err()
                {
                    return Ok(());
                }
            }
            _ => return Err(io::Error::last_os_error()),
        }
    }
}

#[derive(Default)]
struct Decoder {
    wire: Vec<u16>,
    discard_wire: bool,
    encoded_keys: bool,
    raw_paste: bool,
    sequence: Vec<u16>,
    escape_started: Option<Instant>,
    paste: Option<Vec<u16>>,
    surrogate: Option<u16>,
    mouse_buttons: u32,
}
impl Decoder {
    fn for_console() -> Self {
        Self {
            encoded_keys: true,
            ..Self::default()
        }
    }
    fn record(&mut self, record: &INPUT_RECORD) -> Vec<Event> {
        match record.EventType as u32 {
            KEY_EVENT => self.key(unsafe { record.Event.KeyEvent }),
            WINDOW_BUFFER_SIZE_EVENT => crossterm::terminal::size()
                .ok()
                .map(|(cols, rows)| vec![Event::Resize(cols, rows)])
                .unwrap_or_default(),
            FOCUS_EVENT => vec![if unsafe { record.Event.FocusEvent.bSetFocus } != 0 {
                Event::FocusGained
            } else {
                Event::FocusLost
            }],
            MOUSE_EVENT => self
                .mouse(unsafe { record.Event.MouseEvent })
                .into_iter()
                .collect(),
            _ => Vec::new(),
        }
    }
    fn key(&mut self, record: KEY_EVENT_RECORD) -> Vec<Event> {
        if record.wVirtualKeyCode != 0 {
            return self.native_key(record);
        }
        let mut events = Vec::new();
        if record.bKeyDown != 0 {
            for _ in 0..record.wRepeatCount.max(1) {
                self.wire_unit(unsafe { record.uChar.UnicodeChar }, &mut events);
            }
        }
        events
    }
    fn wire_unit(&mut self, unit: u16, events: &mut Vec<Event>) {
        if self.discard_wire {
            if (0x40..=0x7e).contains(&unit) {
                self.discard_wire = false;
            }
            return;
        }
        // Raw and encoded paste framing can both reach the console reader.
        // Once an opener arrived raw, every payload unit is literal until its
        // real closing delimiter, even if it resembles a native key frame.
        if self.raw_paste {
            self.unit(unit, events);
            self.raw_paste = self.paste.is_some();
            return;
        }
        if self.wire.is_empty() {
            if unit == 27 {
                self.wire.push(unit);
                self.escape_started = Some(Instant::now());
            } else {
                self.unit(unit, events);
            }
            return;
        }
        self.wire.push(unit);
        let completed = (self.wire.len() == 2 && unit != 91 && unit != 79)
            || (self.wire.len() > 2 && (0x40..=0x7e).contains(&unit));
        if completed {
            let wire = std::mem::take(&mut self.wire);
            if wire.starts_with(&[27, 91]) && unit == b'_' as u16 {
                if let Some(record) = native_record(&wire) {
                    self.encoded_keys = true;
                    // Unwrap exactly once: VK=0 payload units are semantic
                    // input, never recursively interpreted as wire frames.
                    events.extend(self.native_key(record));
                }
            } else {
                let already_pasting = self.paste.is_some();
                for unit in wire {
                    self.unit(unit, events);
                }
                if !already_pasting && self.paste.is_some() {
                    self.raw_paste = true;
                }
            }
        } else if self.wire.len() > 64 {
            self.wire.clear();
            self.discard_wire = true;
        }
    }
    fn native_key(&mut self, record: KEY_EVENT_RECORD) -> Vec<Event> {
        let unit = unsafe { record.uChar.UnicodeChar };
        let vk = record.wVirtualKeyCode;
        let alt_code = vk == 0x12 && record.bKeyDown == 0 && unit != 0;
        if record.bKeyDown == 0 && !alt_code {
            return Vec::new();
        }
        if matches!(vk, 0x10..=0x12) && !alt_code {
            return Vec::new();
        }
        let state = modifiers(record.dwControlKeyState);
        if (0x60..=0x69).contains(&vk) && state == KeyModifiers::ALT {
            return Vec::new();
        }
        let mut result = Vec::new();
        for _ in 0..record.wRepeatCount.max(1) {
            if self.paste.is_some() || vk == 0 || alt_code {
                if vk != 0 && unit == 0 {
                    continue;
                }
                self.unit(unit, &mut result);
                continue;
            }
            let mut modifiers = modifiers(record.dwControlKeyState);
            // Windows represents AltGr as right Alt + left Ctrl. The Unicode
            // unit is already translated by the user's layout.
            if record.dwControlKeyState & (RIGHT_ALT_PRESSED | LEFT_CTRL_PRESSED)
                == RIGHT_ALT_PRESSED | LEFT_CTRL_PRESSED
                && unit >= 32
            {
                modifiers.remove(KeyModifiers::ALT | KeyModifiers::CONTROL);
            }
            let code = match vk {
                8 => Some(KeyCode::Backspace),
                9 => Some(if modifiers.contains(KeyModifiers::SHIFT) {
                    KeyCode::BackTab
                } else {
                    KeyCode::Tab
                }),
                13 => Some(KeyCode::Enter),
                27 => Some(KeyCode::Esc),
                33 => Some(KeyCode::PageUp),
                34 => Some(KeyCode::PageDown),
                35 => Some(KeyCode::End),
                36 => Some(KeyCode::Home),
                37 => Some(KeyCode::Left),
                38 => Some(KeyCode::Up),
                39 => Some(KeyCode::Right),
                40 => Some(KeyCode::Down),
                45 => Some(KeyCode::Insert),
                46 => Some(KeyCode::Delete),
                112..=135 => Some(KeyCode::F((vk - 111) as u8)),
                32 if unit == 0 && modifiers.contains(KeyModifiers::CONTROL) => {
                    Some(KeyCode::Char(' '))
                }
                _ if unit < 32
                    && (65..=90).contains(&vk)
                    && modifiers.contains(KeyModifiers::CONTROL) =>
                {
                    Some(KeyCode::Char(char::from_u32(u32::from(vk + 32)).unwrap()))
                }
                // ConPTY can keep Ctrl-\ as a native key record with its
                // control unit. Normalize that exact unit just as the VT
                // byte path does; the terminal mode switch binds Ctrl-\.
                _ if unit == 0x1c && modifiers.contains(KeyModifiers::CONTROL) => {
                    Some(KeyCode::Char('\\'))
                }
                _ if unit != 0 => self.scalar(unit).map(KeyCode::Char),
                _ => None,
            };
            if let Some(code) = code {
                result.push(key(code, modifiers));
            }
        }
        result
    }
    fn scalar(&mut self, unit: u16) -> Option<char> {
        if (0xD800..=0xDBFF).contains(&unit) {
            self.surrogate = Some(unit);
            return None;
        }
        if let Some(high) = self.surrogate.take()
            && (0xDC00..=0xDFFF).contains(&unit)
        {
            return char::decode_utf16([high, unit]).next().and_then(Result::ok);
        }
        char::from_u32(u32::from(unit))
    }
    fn unit(&mut self, unit: u16, events: &mut Vec<Event>) {
        if let Some(paste) = self.paste.as_mut() {
            // Retain a separate six-unit delimiter window even after rejecting
            // a too-large paste, so its remainder can never become commands.
            self.sequence.push(unit);
            const END: &[u16] = &[27, 91, 50, 48, 49, 126];
            while !END.starts_with(&self.sequence) {
                if paste.len() < MAX_PASTE_UNITS {
                    paste.push(self.sequence[0]);
                }
                self.sequence.remove(0);
            }
            if self.sequence == END {
                let paste = self.paste.take().unwrap();
                self.sequence.clear();
                events.push(Event::Paste(paste_text(&paste)));
            }
            return;
        }
        if !self.sequence.is_empty() {
            self.sequence.push(unit);
            if self.sequence == [27, 91, 50, 48, 48, 126] {
                self.sequence.clear();
                self.paste = Some(Vec::new());
                return;
            }
            if self.sequence.len() == 2 && unit != 91 && unit != 79 {
                self.sequence.clear();
                if let Some(code) = plain_code(unit) {
                    events.push(key(code, KeyModifiers::ALT));
                }
                return;
            }
            if self.sequence.len() > 2 && (0x40..=0x7e).contains(&unit) {
                if let Some(event) = sequence_event(&self.sequence) {
                    events.push(event);
                }
                self.sequence.clear();
            } else if self.sequence.len() > 64 {
                self.sequence.clear();
            }
            return;
        }
        if unit == 27 {
            self.sequence.push(unit);
            self.escape_started = Some(Instant::now());
            return;
        }
        if let Some(ch) = self.scalar(unit) {
            let (code, modifiers) = match ch {
                '\0' => (KeyCode::Char(' '), KeyModifiers::CONTROL),
                '\t' => (KeyCode::Tab, KeyModifiers::NONE),
                '\r' | '\n' => (KeyCode::Enter, KeyModifiers::NONE),
                '\x7f' | '\x08' => (KeyCode::Backspace, KeyModifiers::NONE),
                '\x01'..='\x1a' => (
                    KeyCode::Char(char::from_u32(ch as u32 + 96).unwrap()),
                    KeyModifiers::CONTROL,
                ),
                '\x1c'..='\x1f' => (
                    KeyCode::Char(char::from_u32(ch as u32 + 64).unwrap()),
                    KeyModifiers::CONTROL,
                ),
                _ => (KeyCode::Char(ch), KeyModifiers::NONE),
            };
            events.push(key(code, modifiers));
        }
    }
    fn escape_timeout(&mut self, now: Instant) -> Option<Event> {
        if self.paste.is_none()
            && self.legacy_escape_pending()
            && self
                .escape_started
                .is_some_and(|at| now.duration_since(at) >= ESCAPE_DELAY)
        {
            self.sequence.clear();
            self.wire.clear();
            return Some(key(KeyCode::Esc, KeyModifiers::NONE));
        }
        None
    }
    fn legacy_escape_pending(&self) -> bool {
        !self.encoded_keys
            && ((self.wire == [27] && self.sequence.is_empty())
                || (self.wire.is_empty() && self.sequence == [27]))
    }
    fn mouse(&mut self, record: MOUSE_EVENT_RECORD) -> Option<Event> {
        let current = record.dwButtonState & 0xffff;
        let changed = self.mouse_buttons ^ current;
        self.mouse_buttons = current;
        let button = |bits| {
            if bits & 1 != 0 {
                MouseButton::Left
            } else if bits & 2 != 0 {
                MouseButton::Right
            } else {
                MouseButton::Middle
            }
        };
        let kind = match record.dwEventFlags {
            0 | DOUBLE_CLICK if changed != 0 => {
                if current & changed != 0 {
                    MouseEventKind::Down(button(changed))
                } else {
                    MouseEventKind::Up(button(changed))
                }
            }
            MOUSE_MOVED => {
                if current == 0 {
                    MouseEventKind::Moved
                } else {
                    MouseEventKind::Drag(button(current))
                }
            }
            MOUSE_WHEELED => {
                if (record.dwButtonState as i32) < 0 {
                    MouseEventKind::ScrollDown
                } else {
                    MouseEventKind::ScrollUp
                }
            }
            MOUSE_HWHEELED => {
                if (record.dwButtonState as i32) < 0 {
                    MouseEventKind::ScrollLeft
                } else {
                    MouseEventKind::ScrollRight
                }
            }
            _ => return None,
        };
        let mut info: CONSOLE_SCREEN_BUFFER_INFO = unsafe { std::mem::zeroed() };
        let output = unsafe { GetStdHandle(STD_OUTPUT_HANDLE) };
        if unsafe { GetConsoleScreenBufferInfo(output, &mut info) } == 0 {
            return None;
        }
        Some(Event::Mouse(MouseEvent {
            kind,
            column: record.dwMousePosition.X.max(0) as u16,
            row: (record.dwMousePosition.Y - info.srWindow.Top).max(0) as u16,
            modifiers: modifiers(record.dwControlKeyState),
        }))
    }
}

fn native_record(wire: &[u16]) -> Option<KEY_EVENT_RECORD> {
    let text = String::from_utf16(wire).ok()?;
    let body = text.strip_prefix("\x1b[")?.strip_suffix('_')?;
    let mut fields = [0u32, 0, 0, 0, 0, 1];
    for (index, value) in body.split(';').enumerate() {
        let field = fields.get_mut(index)?;
        if !value.is_empty() {
            if !value.bytes().all(|byte| byte.is_ascii_digit()) {
                return None;
            }
            *field = value.parse().ok()?;
        }
    }
    if fields[3] > 1 {
        return None;
    }
    Some(KEY_EVENT_RECORD {
        wVirtualKeyCode: fields[0].try_into().ok()?,
        wVirtualScanCode: fields[1].try_into().ok()?,
        uChar: KEY_EVENT_RECORD_0 {
            UnicodeChar: fields[2].try_into().ok()?,
        },
        bKeyDown: fields[3] as i32,
        dwControlKeyState: fields[4],
        wRepeatCount: fields[5].try_into().ok()?,
    })
}

fn key(code: KeyCode, modifiers: KeyModifiers) -> Event {
    Event::Key(KeyEvent::new(code, modifiers))
}

fn paste_text(units: &[u16]) -> String {
    let text = String::from_utf16_lossy(units);
    let mut chars = text.chars().peekable();
    let mut result = String::with_capacity(text.len());
    while let Some(ch) = chars.next() {
        // ConPTY translates pasted line endings into Enter/CR records. The
        // editor indexes LF lines; restore these separators before delivery.
        // A CRLF pair that survives transport remains a CRLF pair.
        result.push(if ch == '\r' && chars.peek() != Some(&'\n') {
            '\n'
        } else {
            ch
        });
    }
    result
}
fn modifiers(state: u32) -> KeyModifiers {
    let mut result = KeyModifiers::NONE;
    if state & SHIFT_PRESSED != 0 {
        result |= KeyModifiers::SHIFT;
    }
    if state & (LEFT_CTRL_PRESSED | RIGHT_CTRL_PRESSED) != 0 {
        result |= KeyModifiers::CONTROL;
    }
    if state & (LEFT_ALT_PRESSED | RIGHT_ALT_PRESSED) != 0 {
        result |= KeyModifiers::ALT;
    }
    result
}
fn plain_code(unit: u16) -> Option<KeyCode> {
    Some(match unit {
        9 => KeyCode::Tab,
        13 => KeyCode::Enter,
        27 => KeyCode::Esc,
        127 => KeyCode::Backspace,
        _ => KeyCode::Char(char::from_u32(u32::from(unit))?),
    })
}
fn sequence_event(units: &[u16]) -> Option<Event> {
    let text = String::from_utf16(units).ok()?;
    let last = text.chars().last()?;
    let body = &text[2..text.len() - 1];
    if let Some(body) = body.strip_prefix('<') {
        let args = body
            .split(';')
            .map(str::parse::<u16>)
            .collect::<Result<Vec<_>, _>>()
            .ok()?;
        let [bits, x, y] = args.as_slice() else {
            return None;
        };
        let mut modifiers = KeyModifiers::NONE;
        if bits & 4 != 0 {
            modifiers |= KeyModifiers::SHIFT;
        }
        if bits & 8 != 0 {
            modifiers |= KeyModifiers::ALT;
        }
        if bits & 16 != 0 {
            modifiers |= KeyModifiers::CONTROL;
        }
        let button = match bits & 3 {
            0 => MouseButton::Left,
            1 => MouseButton::Middle,
            2 => MouseButton::Right,
            _ => MouseButton::Left,
        };
        let kind = if bits & 64 != 0 {
            match bits & 3 {
                0 => MouseEventKind::ScrollUp,
                1 => MouseEventKind::ScrollDown,
                2 => MouseEventKind::ScrollLeft,
                _ => MouseEventKind::ScrollRight,
            }
        } else if last == 'm' {
            MouseEventKind::Up(button)
        } else if last != 'M' {
            return None;
        } else if bits & 32 != 0 {
            if bits & 3 == 3 {
                MouseEventKind::Moved
            } else {
                MouseEventKind::Drag(button)
            }
        } else {
            MouseEventKind::Down(button)
        };
        return Some(Event::Mouse(MouseEvent {
            kind,
            modifiers,
            column: x.checked_sub(1)?,
            row: y.checked_sub(1)?,
        }));
    }
    let args = body
        .split(';')
        .map(str::parse::<u16>)
        .collect::<Result<Vec<_>, _>>()
        .unwrap_or_default();
    let value = args.get(1).copied().unwrap_or(1).saturating_sub(1);
    let mut modifiers = KeyModifiers::NONE;
    if value & 1 != 0 {
        modifiers |= KeyModifiers::SHIFT;
    }
    if value & 2 != 0 {
        modifiers |= KeyModifiers::ALT;
    }
    if value & 4 != 0 {
        modifiers |= KeyModifiers::CONTROL;
    }
    let code = match last {
        'A' => KeyCode::Up,
        'B' => KeyCode::Down,
        'C' => KeyCode::Right,
        'D' => KeyCode::Left,
        'H' => KeyCode::Home,
        'F' => KeyCode::End,
        'P'..='S' => KeyCode::F(last as u8 - b'P' + 1),
        'Z' => {
            modifiers |= KeyModifiers::SHIFT;
            KeyCode::BackTab
        }
        'I' if body.is_empty() => return Some(Event::FocusGained),
        'O' if body.is_empty() => return Some(Event::FocusLost),
        '~' => match args.first()? {
            1 | 7 => KeyCode::Home,
            2 => KeyCode::Insert,
            3 => KeyCode::Delete,
            4 | 8 => KeyCode::End,
            5 => KeyCode::PageUp,
            6 => KeyCode::PageDown,
            n => KeyCode::F(match n {
                11..=15 => (*n - 10) as u8,
                17..=21 => (*n - 11) as u8,
                23..=24 => (*n - 12) as u8,
                _ => return None,
            }),
        },
        _ => return None,
    };
    Some(key(code, modifiers))
}

#[cfg(test)]
mod tests;
