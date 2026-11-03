// SPDX-License-Identifier: MPL-2.0

//! What readers granted Write type into terminals: lines, and the Enter that
//! follows them only after a visible delay, and only if the terminal still
//! shows what the agent read and nothing else was typed there meanwhile.

use super::{WorkspaceHost, context::Reader, context_reads::check_revision};
use crate::{
    plugin::application::{Error, ErrorCode as Code},
    terminal::{TerminalId, proposal::Text},
    workspace::context::{
        storage,
        wire::{self, Scope},
    },
};
use serde_json::{Value, json};
use std::time::{Duration, Instant};

/// Finished Enters kept for their outcome, across all readers.
const RETAINED_ENTERS: usize = 64;
/// How long a due Enter waits for one more pass of the event loop, so that
/// output or input already queued when it fell due is seen before it is
/// pressed.
const ENTER_SETTLE: Duration = Duration::from_millis(100);

/// An Enter a reader asked for.
pub(super) struct Enter {
    owner: u64,
    terminal: TerminalId,
    /// The terminal's read revision the agent looked at.
    revision: u64,
    /// The input generation when it was asked for. Any key, paste or line
    /// typed after that moves it on.
    generation: u64,
    attachment: u64,
    due: Instant,
    /// When a pass first found it due with nothing changed. It is pressed on
    /// a later pass, after `ENTER_SETTLE`.
    armed: Option<Instant>,
    state: &'static str,
    finished: Option<Instant>,
}

impl Enter {
    fn waiting(&self) -> bool {
        self.state == "waiting"
    }
}

impl WorkspaceHost {
    /// Queues one line for `terminal`, never followed by Enter, only while the
    /// person's frontend is attached.
    pub(super) fn context_write_terminal(
        &mut self,
        reader: &mut Reader,
        terminal: TerminalId,
        text: &str,
    ) -> Result<Value, Error> {
        let attachment = self.app.plugins.attachment_generation;
        if !self.app.plugins.frontend_attached {
            return Err(Error::new(
                Code::NoFrontend,
                "Attach to the workspace to write to its terminals",
            ));
        }
        let text = Text::new(text)
            .map_err(|_| Error::new(Code::InvalidArgument, "Invalid terminal text"))?;
        let session = self
            .app
            .terminals
            .get_mut(terminal)
            .filter(|t| t.live())
            .ok_or_else(|| Error::new(Code::Closed, "Terminal is not live"))?;
        let delivery = session
            .queue_agent_text(&text)
            .map_err(|_| Error::new(Code::Unavailable, "Terminal cannot accept text"))?;
        reader.writes.retain(|(delivery, _)| {
            use crate::terminal::proposal::DeliveryState as D;
            matches!(delivery.state(), D::Queued | D::Writing)
        });
        reader.writes.push((delivery, attachment));
        Ok(json!({"state":"queued","chars":text.as_str().chars().count()}))
    }

    /// Schedules one Enter, refused at once when the terminal no longer
    /// shows `expected` revision.
    pub(super) fn context_request_enter(
        &mut self,
        owner: u64,
        terminal: TerminalId,
        expected: &str,
    ) -> Result<Value, Error> {
        if !self.app.plugins.frontend_attached {
            return Err(Error::new(
                Code::NoFrontend,
                "Attach to the workspace to press Enter in its terminals",
            ));
        }
        let session = self
            .app
            .terminals
            .get(terminal)
            .filter(|t| t.live())
            .ok_or_else(|| Error::new(Code::Closed, "Terminal is not live"))?;
        check_revision(expected, session.read_revision())?;
        let waiting = || self.context.enters.values().filter(|e| e.waiting());
        if waiting().any(|e| e.terminal == terminal) {
            return Err(Error::new(
                Code::Conflict,
                "An Enter is already waiting for this terminal",
            ));
        }
        if waiting().count() >= wire::MAX_HOST_ENTERS
            || waiting().filter(|e| e.owner == owner).count() >= wire::MAX_ENTERS
        {
            return Err(Error::new(Code::LimitExceeded, "Enter limit reached"));
        }
        while self.context.enters.len() >= RETAINED_ENTERS {
            let oldest = self
                .context
                .enters
                .iter()
                .filter(|(_, e)| !e.waiting())
                .min_by_key(|(_, e)| e.finished)
                .map(|(id, _)| id.clone());
            match oldest {
                Some(id) => self.context.enters.remove(&id),
                None => break,
            };
        }
        let id = storage::random_token()
            .map_err(|_| Error::new(Code::Unavailable, "Cannot allocate an Enter"))?;
        let enter = Enter {
            owner,
            terminal,
            revision: session.read_revision(),
            generation: session.input_generation(),
            attachment: self.app.plugins.attachment_generation,
            due: Instant::now() + Duration::from_millis(wire::ENTER_DELAY_MILLIS),
            armed: None,
            state: "waiting",
            finished: None,
        };
        self.context.enters.insert(id.clone(), enter);
        if let Some(session) = self.app.terminals.get_mut(terminal) {
            session.set_agent_enter_pending(true);
        }
        self.app.plugins.presentation_dirty = true;
        Ok(json!({"enter":id,"state":"waiting","delay_ms":wire::ENTER_DELAY_MILLIS}))
    }

    pub(super) fn context_enter_status(&self, owner: u64, enter: &str) -> Result<Value, Error> {
        self.context
            .enters
            .get(enter)
            .filter(|e| e.owner == owner)
            .map(|e| json!({"enter":enter,"state":e.state}))
            .ok_or_else(|| Error::new(Code::NotFound, "Unknown Enter"))
    }

    /// Presses each Enter that has waited out its delay with nothing
    /// changed, and drops every one whose terminal showed or received
    /// anything new, or whose reader, grant, frontend or terminal went away.
    /// Text still queued for a frontend that has gone is cancelled too. Runs
    /// where every reader is in the host's table.
    pub(super) fn context_advance_enters(&mut self) {
        let now = Instant::now();
        let attached = self
            .app
            .plugins
            .frontend_attached
            .then_some(self.app.plugins.attachment_generation);
        for reader in self.context.readers.values_mut() {
            reader.writes.retain(|(delivery, attachment)| {
                if Some(*attachment) != attached {
                    delivery.cancel();
                }
                use crate::terminal::proposal::DeliveryState as D;
                matches!(delivery.state(), D::Queued | D::Writing)
            });
        }
        let ids = self
            .context
            .enters
            .iter()
            .filter(|(_, e)| e.waiting())
            .map(|(id, _)| id.clone())
            .collect::<Vec<_>>();
        for id in ids {
            let enter = &self.context.enters[&id];
            let (owner, terminal, revision, generation, attachment, due, armed) = (
                enter.owner,
                enter.terminal,
                enter.revision,
                enter.generation,
                enter.attachment,
                enter.due,
                enter.armed,
            );
            let allowed = self.context_granted(owner, Scope::TerminalWrite)
                && self.app.plugins.frontend_attached
                && attachment == self.app.plugins.attachment_generation;
            let state = match self.app.terminals.get_mut(terminal) {
                Some(session) if allowed && session.live() => {
                    if session.read_revision() != revision
                        || session.input_generation() != generation
                    {
                        "stale"
                    } else if now < due {
                        continue;
                    } else if armed.is_none_or(|armed| now < armed + ENTER_SETTLE) {
                        let enter = self.context.enters.get_mut(&id).expect("listed Enter");
                        enter.armed.get_or_insert(now);
                        continue;
                    } else if session.press_agent_enter(generation) {
                        "sent"
                    } else {
                        "stale"
                    }
                }
                _ => "cancelled",
            };
            self.context_finish_enter(&id, state, now);
        }
        self.context.enters.retain(|_, e| {
            e.finished.is_none_or(|finished| {
                now.duration_since(finished) < Duration::from_secs(wire::ENTER_RECORD_SECONDS)
            })
        });
    }

    /// Cancels the Enters a reader that has gone was waiting for, and drops
    /// their records, which nobody can ask about any more.
    pub(super) fn context_cancel_enters(&mut self, owner: u64) {
        let now = Instant::now();
        let ids = self
            .context
            .enters
            .iter()
            .filter(|(_, e)| e.owner == owner)
            .map(|(id, _)| id.clone())
            .collect::<Vec<_>>();
        for id in ids {
            if self.context.enters[&id].waiting() {
                self.context_finish_enter(&id, "cancelled", now);
            }
            self.context.enters.remove(&id);
        }
    }

    fn context_finish_enter(&mut self, id: &str, state: &'static str, now: Instant) {
        let Some(enter) = self.context.enters.get_mut(id) else {
            return;
        };
        enter.state = state;
        enter.finished = Some(now);
        let terminal = enter.terminal;
        if let Some(session) = self.app.terminals.get_mut(terminal) {
            session.set_agent_enter_pending(false);
        }
        self.app.plugins.presentation_dirty = true;
    }

    /// When the next waiting Enter is due. Output and input wake the event
    /// loop by themselves, which is when one goes stale.
    pub(super) fn context_enter_delay(&self) -> Option<Duration> {
        let now = Instant::now();
        self.context
            .enters
            .values()
            .filter(|e| e.waiting())
            .map(|e| {
                e.armed
                    .map_or(e.due, |armed| armed + ENTER_SETTLE)
                    .saturating_duration_since(now)
            })
            .min()
    }

    /// The state of the Enter with handle `id`, for tests.
    #[cfg(test)]
    pub(super) fn context_enter_state(&self, id: &str) -> Option<&'static str> {
        self.context.enters.get(id).map(|e| e.state)
    }

    /// Moves every waiting Enter's due time, and the end of the settling
    /// pass of one already armed, into the past. A first pass then arms an
    /// Enter, and a pass after a second call presses it.
    #[cfg(test)]
    pub(super) fn context_expire_enter_delays(&mut self) {
        let past = Instant::now() - ENTER_SETTLE - Duration::from_millis(1);
        for enter in self.context.enters.values_mut() {
            enter.due = past;
            if enter.armed.is_some() {
                enter.armed = Some(past);
            }
        }
    }
}
