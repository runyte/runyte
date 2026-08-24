// SPDX-License-Identifier: MPL-2.0

//! Reports Crossterm key identities under Runyte's macOS keyboard profile.
//!
//! This is a development probe, not an editor entry point. It deliberately
//! records key metadata only and never records terminal text or clipboard
//! contents.

use std::io::{self, Write};

use anyhow::{Context, Result};
use crossterm::{
    ExecutableCommand,
    event::{
        Event, KeyCode, KeyEventKind, KeyModifiers, KeyboardEnhancementFlags,
        PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags, read,
    },
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};

struct TerminalGuard;

impl TerminalGuard {
    fn enter() -> Result<Self> {
        enable_raw_mode().context("failed to enable raw mode")?;
        let mut output = io::stdout();
        if let Err(error) = output.execute(EnterAlternateScreen) {
            let _ = disable_raw_mode();
            return Err(error).context("failed to enter the alternate screen");
        }
        let flags = KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
            | KeyboardEnhancementFlags::REPORT_ALTERNATE_KEYS;
        if let Err(error) = output.execute(PushKeyboardEnhancementFlags(flags)) {
            let _ = output.execute(LeaveAlternateScreen);
            let _ = disable_raw_mode();
            return Err(error).context("failed to enable keyboard disambiguation");
        }
        Ok(Self)
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let mut output = io::stdout();
        let _ = output.execute(PopKeyboardEnhancementFlags);
        let _ = output.execute(LeaveAlternateScreen);
        let _ = disable_raw_mode();
    }
}

fn main() -> Result<()> {
    let terminal = [
        ("TERM", std::env::var("TERM").ok()),
        ("TERM_PROGRAM", std::env::var("TERM_PROGRAM").ok()),
        (
            "TERM_PROGRAM_VERSION",
            std::env::var("TERM_PROGRAM_VERSION").ok(),
        ),
    ];
    let mut events = Vec::new();

    {
        let _guard = TerminalGuard::enter()?;
        let mut output = io::stdout();
        write!(
            output,
            "Press Ctrl-h, Ctrl-j, Ctrl-k, Ctrl-l, then Ctrl-w followed by h.\r\n\
             Press plain q when finished. No typed text is recorded.\r\n"
        )?;
        output.flush()?;

        loop {
            let Event::Key(event) = read().context("failed to read a terminal event")? else {
                continue;
            };
            if event.kind == KeyEventKind::Press
                && event.code == KeyCode::Char('q')
                && event.modifiers == KeyModifiers::NONE
            {
                break;
            }
            events.push(format!("{event:?}"));
        }
    }

    for (name, value) in terminal {
        println!("{name}={}", value.as_deref().unwrap_or("<unset>"));
    }
    for (index, event) in events.iter().enumerate() {
        println!("{}: {event}", index + 1);
    }
    Ok(())
}
