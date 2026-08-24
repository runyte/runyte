// SPDX-License-Identifier: MPL-2.0

//! The byte-level escape-sequence state machine.
//!
//! Bytes in, [`Action`]s out. It knows nothing about grids, cursors, or
//! colours, which is what lets it be tested against fixtures rather than
//! against a live child process.
//!
//! The shape follows the DEC parser Paul Williams documented: a ground state
//! that prints, a handful of collecting states for escape, CSI, OSC and DCS,
//! and one rule that an unexpected byte abandons the sequence rather than
//! corrupting the one after it.

/// One thing the byte stream asked for.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Action {
    /// A decoded character to place at the cursor.
    Print(char),
    /// A C0 control byte with its own meaning.
    Execute(u8),
    /// `ESC` with intermediates and a final byte, e.g. `ESC 7`.
    Escape {
        intermediates: Vec<u8>,
        final_byte: u8,
    },
    /// `CSI`: parameters, each with its colon-separated sub-parameters.
    Csi {
        /// `?`, `>` or `<` when the sequence is a private one.
        private: Option<u8>,
        parameters: Vec<Vec<u16>>,
        intermediates: Vec<u8>,
        final_byte: u8,
    },
    /// An operating-system command, split on its semicolons.
    Osc(Vec<Vec<u8>>),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum State {
    Ground,
    Escape,
    EscapeIntermediate,
    CsiEntry,
    CsiParameter,
    CsiIntermediate,
    CsiIgnore,
    Osc,
    /// A device-control string, consumed and discarded up to its terminator.
    DcsIgnore,
    /// Inside a string state, having just seen `ESC`; `\` ends it.
    StringEscape,
}

/// How many bytes one sequence may collect before it is abandoned.
///
/// A runaway OSC — a title containing a byte that never terminates it — must
/// not become unbounded memory. The limit is far above any real title or
/// clipboard payload a child sends.
const SEQUENCE_LIMIT: usize = 8 * 1024;

#[derive(Debug)]
pub struct Parser {
    state: State,
    intermediates: Vec<u8>,
    private: Option<u8>,
    parameters: Vec<Vec<u16>>,
    current: Vec<u16>,
    current_value: Option<u16>,
    osc: Vec<Vec<u8>>,
    /// Stored payload bytes and structural fields in the active sequence.
    /// Counting both is what bounds empty OSC fields and CSI subparameters.
    sequence_size: usize,
    /// Partial UTF-8 sequence carried between calls, since a child's write may
    /// split a character across two reads.
    utf8: Vec<u8>,
    utf8_needed: usize,
}

impl Default for Parser {
    fn default() -> Self {
        Self::new()
    }
}

impl Parser {
    pub fn new() -> Self {
        Self {
            state: State::Ground,
            intermediates: Vec::new(),
            private: None,
            parameters: Vec::new(),
            current: Vec::new(),
            current_value: None,
            osc: Vec::new(),
            sequence_size: 0,
            utf8: Vec::new(),
            utf8_needed: 0,
        }
    }

    /// Feeds bytes, calling `apply` once for each action they produce.
    pub fn advance(&mut self, bytes: &[u8], mut apply: impl FnMut(Action)) {
        for &byte in bytes {
            self.step(byte, &mut apply);
        }
    }

    fn step(&mut self, byte: u8, apply: &mut impl FnMut(Action)) {
        // A control byte inside a partial character means the child wrote a
        // malformed sequence. Emit the replacement and handle the control
        // rather than swallowing it.
        if !self.utf8.is_empty() && !(0x80..0xc0).contains(&byte) {
            self.utf8.clear();
            self.utf8_needed = 0;
            apply(Action::Print(char::REPLACEMENT_CHARACTER));
        }
        match self.state {
            State::Ground => self.ground(byte, apply),
            State::Escape => self.escape(byte, apply),
            State::EscapeIntermediate => self.escape_intermediate(byte, apply),
            State::CsiEntry | State::CsiParameter => self.csi_parameter(byte, apply),
            State::CsiIntermediate => self.csi_intermediate(byte, apply),
            State::CsiIgnore => self.csi_ignore(byte),
            State::Osc => self.osc(byte, apply),
            State::DcsIgnore => self.dcs(byte),
            State::StringEscape => self.string_escape(byte, apply),
        }
    }

    fn reset_sequence(&mut self) {
        self.intermediates.clear();
        self.private = None;
        self.parameters.clear();
        self.current.clear();
        self.current_value = None;
        self.sequence_size = 0;
    }

    fn reserve_sequence(&mut self, bytes: usize) -> bool {
        let Some(size) = self.sequence_size.checked_add(bytes) else {
            return false;
        };
        if size > SEQUENCE_LIMIT {
            return false;
        }
        self.sequence_size = size;
        true
    }

    fn abandon_sequence(&mut self, state: State) {
        self.reset_sequence();
        self.osc.clear();
        self.state = state;
    }

    fn ground(&mut self, byte: u8, apply: &mut impl FnMut(Action)) {
        if !self.utf8.is_empty() {
            self.utf8.push(byte);
            if self.utf8.len() >= self.utf8_needed {
                match std::str::from_utf8(&self.utf8) {
                    Ok(text) => {
                        for character in text.chars() {
                            apply(Action::Print(character));
                        }
                    }
                    Err(_) => apply(Action::Print(char::REPLACEMENT_CHARACTER)),
                }
                self.utf8.clear();
                self.utf8_needed = 0;
            }
            return;
        }
        match byte {
            0x1b => {
                self.reset_sequence();
                self.state = State::Escape;
            }
            0x00..=0x1f | 0x7f => apply(Action::Execute(byte)),
            0x20..=0x7e => apply(Action::Print(byte as char)),
            0xc0..=0xdf => {
                self.utf8.push(byte);
                self.utf8_needed = 2;
            }
            0xe0..=0xef => {
                self.utf8.push(byte);
                self.utf8_needed = 3;
            }
            0xf0..=0xf7 => {
                self.utf8.push(byte);
                self.utf8_needed = 4;
            }
            _ => apply(Action::Print(char::REPLACEMENT_CHARACTER)),
        }
    }

    fn escape(&mut self, byte: u8, apply: &mut impl FnMut(Action)) {
        match byte {
            0x5b => self.state = State::CsiEntry,
            0x5d => {
                self.osc = vec![Vec::new()];
                self.sequence_size = std::mem::size_of::<Vec<u8>>();
                self.state = State::Osc;
            }
            0x50 => self.state = State::DcsIgnore,
            // SOS, PM and APC are string states with nothing here that wants
            // them; consume to the terminator like a DCS.
            0x58 | 0x5e | 0x5f => self.state = State::DcsIgnore,
            0x20..=0x2f => {
                if self.reserve_sequence(1) {
                    self.intermediates.push(byte);
                    self.state = State::EscapeIntermediate;
                } else {
                    self.abandon_sequence(State::Ground);
                }
            }
            0x30..=0x7e => {
                apply(Action::Escape {
                    intermediates: std::mem::take(&mut self.intermediates),
                    final_byte: byte,
                });
                self.sequence_size = 0;
                self.state = State::Ground;
            }
            0x18 | 0x1a => {
                apply(Action::Execute(byte));
                self.reset_sequence();
                self.state = State::Ground;
            }
            0x1b => self.reset_sequence(),
            _ => {
                self.reset_sequence();
                self.state = State::Ground;
            }
        }
    }

    fn escape_intermediate(&mut self, byte: u8, apply: &mut impl FnMut(Action)) {
        match byte {
            0x20..=0x2f => {
                if self.reserve_sequence(1) {
                    self.intermediates.push(byte);
                } else {
                    self.abandon_sequence(State::Ground);
                }
            }
            0x30..=0x7e => {
                apply(Action::Escape {
                    intermediates: std::mem::take(&mut self.intermediates),
                    final_byte: byte,
                });
                self.sequence_size = 0;
                self.state = State::Ground;
            }
            _ => {
                self.reset_sequence();
                self.state = State::Ground;
            }
        }
    }

    fn push_parameter(&mut self) {
        let value = self.current_value.take().unwrap_or(0);
        self.current.push(value);
        let parameter = std::mem::take(&mut self.current);
        self.parameters.push(parameter);
    }

    fn csi_parameter(&mut self, byte: u8, apply: &mut impl FnMut(Action)) {
        match byte {
            0x30..=0x39 => {
                let digit = u16::from(byte - b'0');
                let value = self.current_value.unwrap_or(0);
                // Saturating rather than wrapping: a parameter longer than a
                // terminal could act on should clamp, not alias to a small one.
                self.current_value = Some(value.saturating_mul(10).saturating_add(digit));
                self.state = State::CsiParameter;
            }
            0x3a => {
                if self.reserve_sequence(std::mem::size_of::<u16>()) {
                    let value = self.current_value.take().unwrap_or(0);
                    self.current.push(value);
                    self.state = State::CsiParameter;
                } else {
                    self.abandon_sequence(State::CsiIgnore);
                }
            }
            0x3b => {
                if self
                    .reserve_sequence(std::mem::size_of::<u16>() + std::mem::size_of::<Vec<u16>>())
                {
                    self.push_parameter();
                    self.state = State::CsiParameter;
                } else {
                    self.abandon_sequence(State::CsiIgnore);
                }
            }
            0x3c..=0x3f if self.state == State::CsiEntry => {
                self.private = Some(byte);
                self.state = State::CsiParameter;
            }
            0x20..=0x2f => {
                if self.reserve_sequence(1) {
                    self.intermediates.push(byte);
                    self.state = State::CsiIntermediate;
                } else {
                    self.abandon_sequence(State::CsiIgnore);
                }
            }
            0x40..=0x7e => {
                if self.current_value.is_some() || !self.current.is_empty() {
                    self.push_parameter();
                }
                apply(Action::Csi {
                    private: self.private.take(),
                    parameters: std::mem::take(&mut self.parameters),
                    intermediates: std::mem::take(&mut self.intermediates),
                    final_byte: byte,
                });
                self.reset_sequence();
                self.state = State::Ground;
            }
            0x00..=0x17 | 0x19 | 0x1c..=0x1f => apply(Action::Execute(byte)),
            0x1b => {
                self.reset_sequence();
                self.state = State::Escape;
            }
            _ => self.state = State::CsiIgnore,
        }
        if self.parameters.len() > 32 {
            self.state = State::CsiIgnore;
        }
    }

    fn csi_intermediate(&mut self, byte: u8, apply: &mut impl FnMut(Action)) {
        match byte {
            0x20..=0x2f => {
                if self.reserve_sequence(1) {
                    self.intermediates.push(byte);
                } else {
                    self.abandon_sequence(State::CsiIgnore);
                }
            }
            0x40..=0x7e => {
                if self.current_value.is_some() || !self.current.is_empty() {
                    self.push_parameter();
                }
                apply(Action::Csi {
                    private: self.private.take(),
                    parameters: std::mem::take(&mut self.parameters),
                    intermediates: std::mem::take(&mut self.intermediates),
                    final_byte: byte,
                });
                self.reset_sequence();
                self.state = State::Ground;
            }
            0x00..=0x17 | 0x19 | 0x1c..=0x1f => apply(Action::Execute(byte)),
            _ => self.state = State::CsiIgnore,
        }
    }

    fn csi_ignore(&mut self, byte: u8) {
        match byte {
            0x40..=0x7e => {
                self.reset_sequence();
                self.state = State::Ground;
            }
            0x1b => {
                self.reset_sequence();
                self.state = State::Escape;
            }
            _ => {}
        }
    }

    fn osc(&mut self, byte: u8, apply: &mut impl FnMut(Action)) {
        match byte {
            0x07 => {
                apply(Action::Osc(std::mem::take(&mut self.osc)));
                self.sequence_size = 0;
                self.state = State::Ground;
            }
            0x1b => self.state = State::StringEscape,
            0x3b => {
                if self.reserve_sequence(std::mem::size_of::<Vec<u8>>()) {
                    self.osc.push(Vec::new());
                } else {
                    self.abandon_sequence(State::Ground);
                }
            }
            0x00..=0x06 | 0x08..=0x1a | 0x1c..=0x1f => {
                self.osc.clear();
                self.sequence_size = 0;
                self.state = State::Ground;
            }
            _ => {
                if !self.reserve_sequence(1) {
                    self.abandon_sequence(State::Ground);
                } else if let Some(last) = self.osc.last_mut() {
                    last.push(byte);
                }
            }
        }
    }

    fn dcs(&mut self, byte: u8) {
        if byte == 0x1b {
            self.state = State::StringEscape;
        }
    }

    fn string_escape(&mut self, byte: u8, apply: &mut impl FnMut(Action)) {
        if byte == b'\\' {
            if !self.osc.is_empty() {
                apply(Action::Osc(std::mem::take(&mut self.osc)));
            }
            self.sequence_size = 0;
            self.state = State::Ground;
        } else {
            // Not a terminator after all: the child began a new escape inside
            // a string. Abandon the string and read this as the escape.
            self.osc.clear();
            self.reset_sequence();
            self.state = State::Escape;
            self.escape(byte, apply);
        }
    }
}

/// The first sub-parameter of parameter `index`, or `default` when absent or
/// written as an empty field.
pub fn parameter(parameters: &[Vec<u16>], index: usize, default: u16) -> u16 {
    match parameters.get(index).and_then(|values| values.first()) {
        Some(&0) | None => default,
        Some(&value) => value,
    }
}

/// The first sub-parameter of parameter `index` with zero kept as zero.
pub fn raw_parameter(parameters: &[Vec<u16>], index: usize) -> u16 {
    parameters
        .get(index)
        .and_then(|values| values.first())
        .copied()
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn actions(bytes: &[u8]) -> Vec<Action> {
        let mut parser = Parser::new();
        let mut collected = Vec::new();
        parser.advance(bytes, |action| collected.push(action));
        collected
    }

    #[test]
    fn plain_text_prints_one_action_per_character() {
        assert_eq!(actions(b"hi"), vec![Action::Print('h'), Action::Print('i')]);
    }

    #[test]
    fn a_csi_carries_its_parameters_private_marker_and_final_byte() {
        assert_eq!(
            actions(b"\x1b[?25l"),
            vec![Action::Csi {
                private: Some(b'?'),
                parameters: vec![vec![25]],
                intermediates: Vec::new(),
                final_byte: b'l',
            }]
        );
    }

    #[test]
    fn an_empty_parameter_field_is_preserved_as_zero() {
        assert_eq!(
            actions(b"\x1b[;5H"),
            vec![Action::Csi {
                private: None,
                parameters: vec![vec![0], vec![5]],
                intermediates: Vec::new(),
                final_byte: b'H',
            }]
        );
    }

    #[test]
    fn colon_separated_sub_parameters_stay_with_their_parameter() {
        assert_eq!(
            actions(b"\x1b[38:2::1:2:3m"),
            vec![Action::Csi {
                private: None,
                parameters: vec![vec![38, 2, 0, 1, 2, 3]],
                intermediates: Vec::new(),
                final_byte: b'm',
            }]
        );
    }

    #[test]
    fn an_osc_ends_on_bel_or_on_string_terminator() {
        assert_eq!(
            actions(b"\x1b]0;title\x07"),
            vec![Action::Osc(vec![b"0".to_vec(), b"title".to_vec()])]
        );
        assert_eq!(
            actions(b"\x1b]0;title\x1b\\"),
            vec![Action::Osc(vec![b"0".to_vec(), b"title".to_vec()])]
        );
    }

    #[test]
    fn a_device_control_string_is_consumed_whole() {
        assert_eq!(
            actions(b"\x1bP1$r0m\x1b\\ok"),
            vec![Action::Print('o'), Action::Print('k')]
        );
    }

    #[test]
    fn a_character_split_across_two_writes_is_decoded_once() {
        let mut parser = Parser::new();
        let mut collected = Vec::new();
        parser.advance(&[0xe6], |action| collected.push(action));
        assert!(collected.is_empty());
        parser.advance(&[0xbc, 0xa2], |action| collected.push(action));
        assert_eq!(collected, vec![Action::Print('漢')]);
    }

    #[test]
    fn a_truncated_character_becomes_one_replacement_and_the_control_survives() {
        assert_eq!(
            actions(&[0xe6, b'\n']),
            vec![
                Action::Print(char::REPLACEMENT_CHARACTER),
                Action::Execute(b'\n')
            ]
        );
    }

    #[test]
    fn an_escape_inside_a_sequence_abandons_it() {
        assert_eq!(
            actions(b"\x1b[12\x1b[H"),
            vec![Action::Csi {
                private: None,
                parameters: Vec::new(),
                intermediates: Vec::new(),
                final_byte: b'H',
            }]
        );
    }

    /// A title a child never terminates must not become unbounded memory.
    /// The sequence is abandoned at the limit and the stream resumes as text,
    /// which is recoverable; growing the buffer forever is not.
    #[test]
    fn an_unterminated_osc_is_abandoned_rather_than_grown_without_bound() {
        let mut bytes = b"\x1b]0;".to_vec();
        let overrun = 16;
        bytes.extend(std::iter::repeat_n(b'x', SEQUENCE_LIMIT + overrun));
        bytes.extend_from_slice(b"ok");
        let actions = actions(&bytes);
        assert!(
            !actions
                .iter()
                .any(|action| matches!(action, Action::Osc(_)))
        );
        assert!(actions.len() < 128);
        assert_eq!(
            &actions[actions.len() - 2..],
            &[Action::Print('o'), Action::Print('k')]
        );
    }

    #[test]
    fn empty_osc_fields_are_counted_toward_the_sequence_limit() {
        let mut parser = Parser::new();
        let bytes = std::iter::once(0x1b)
            .chain(std::iter::once(b']'))
            .chain(std::iter::repeat_n(b';', SEQUENCE_LIMIT + 1))
            .collect::<Vec<_>>();
        parser.advance(&bytes, |_| {});

        assert_eq!(parser.state, State::Ground);
        assert!(parser.osc.is_empty());
        assert_eq!(parser.sequence_size, 0);
    }

    #[test]
    fn csi_subparameters_are_counted_toward_the_sequence_limit() {
        let mut parser = Parser::new();
        let bytes = std::iter::once(0x1b)
            .chain(std::iter::once(b'['))
            .chain(std::iter::repeat_n(b':', SEQUENCE_LIMIT + 1))
            .collect::<Vec<_>>();
        parser.advance(&bytes, |_| {});

        assert_eq!(parser.state, State::CsiIgnore);
        assert!(parser.current.is_empty());
        assert!(parser.parameters.is_empty());
        assert_eq!(parser.sequence_size, 0);
    }

    #[test]
    fn escape_intermediates_are_counted_toward_the_sequence_limit() {
        let mut parser = Parser::new();
        let bytes = std::iter::once(0x1b)
            .chain(std::iter::repeat_n(b' ', SEQUENCE_LIMIT + 1))
            .collect::<Vec<_>>();
        parser.advance(&bytes, |_| {});

        assert_eq!(parser.state, State::Ground);
        assert!(parser.intermediates.is_empty());
        assert_eq!(parser.sequence_size, 0);
    }
}
