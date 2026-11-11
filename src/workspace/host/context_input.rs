// SPDX-License-Identifier: MPL-2.0

//! What readers granted Write type into terminals: lines, and the Enter that
//! submits them. Nothing is typed into a terminal the person typed in during
//! the last [`wire::TYPING_QUIET_MILLIS`]. A line's own Enter follows once the
//! line has been written; a separate Enter needs the terminal to still show
//! what the agent read. Any input typed meanwhile drops either.

use super::{WorkspaceHost, context::Reader, context_reads::check_revision};
use crate::{
    plugin::application::{Error, ErrorCode as Code},
    terminal::{
        TerminalId, TerminalSession,
        proposal::{Delivery, DeliveryState, Text},
    },
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
/// pressed. After a line, it also keeps the Enter from arriving in the same
/// read as the paste, which some prompts would take as part of it.
const ENTER_SETTLE: Duration = Duration::from_millis(100);
/// How often a line's Enter checks whether the line has been written.
pub(super) const DELIVERY_POLL: Duration = Duration::from_millis(20);
/// How long a line's Enter waits for the line to be written. A child that
/// stops reading its input leaves the line queued; an Enter pressed long
/// after would land on a screen the agent never saw.
const LINE_DEADLINE: Duration = Duration::from_secs(4);

/// An Enter a reader asked for.
pub(super) struct Enter {
    owner: u64,
    terminal: TerminalId,
    /// The terminal's read revision the agent looked at, for a separate
    /// Enter. A line's own Enter has none: the line's echo is output.
    revision: Option<u64>,
    /// The line this Enter submits, pressed only once it has been written,
    /// and by when it must have been.
    after: Option<(Delivery, Instant)>,
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
    /// Whether its line is still on its way to the terminal.
    fn delivering(&self) -> bool {
        self.after.as_ref().is_some_and(|(line, _)| {
            matches!(line.state(), DeliveryState::Queued | DeliveryState::Writing)
        })
    }
}

/// Refuses a terminal the person typed in during the last
/// [`wire::TYPING_QUIET_MILLIS`], saying how much longer to wait.
fn typing_quiet(session: &TerminalSession) -> Result<(), Error> {
    let quiet = Duration::from_millis(wire::TYPING_QUIET_MILLIS);
    match session.since_typed() {
        Some(since) if since < quiet => {
            let wait = (quiet - since).as_millis().div_ceil(1000);
            Err(Error::new(
                Code::Busy,
                &format!(
                    "The person typed in this terminal {}s ago. Wait {wait}s more, read it again, and type only if it is still safe",
                    since.as_secs()
                ),
            ))
        }
        _ => Ok(()),
    }
}

impl WorkspaceHost {
    /// Queues one line for `terminal`, only while the person's frontend is
    /// attached and they have not typed there recently. With `enter`, the
    /// line is submitted once it has been written.
    pub(super) fn context_write_terminal(
        &mut self,
        owner: u64,
        reader: &mut Reader,
        terminal: TerminalId,
        text: &str,
        enter: bool,
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
            .get(terminal)
            .filter(|t| t.live())
            .ok_or_else(|| Error::new(Code::Closed, "Terminal is not live"))?;
        typing_quiet(session)?;
        // Everything that could refuse the Enter is checked before the line
        // is queued, so a line asked to be submitted is never left unsent.
        let id = enter
            .then(|| self.context_admit_enter(owner, terminal))
            .transpose()?;
        let session = self
            .app
            .terminals
            .get_mut(terminal)
            .expect("terminal checked above");
        let delivery = session
            .queue_agent_text(&text)
            .map_err(|_| Error::new(Code::Unavailable, "Terminal cannot accept text"))?;
        let generation = session.input_generation();
        reader.writes.retain(|(delivery, _)| {
            matches!(
                delivery.state(),
                DeliveryState::Queued | DeliveryState::Writing
            )
        });
        reader.writes.push((delivery.clone(), attachment));
        let chars = text.as_str().chars().count();
        let Some(id) = id else {
            return Ok(json!({"state":"queued","chars":chars}));
        };
        self.context_schedule_enter(
            id.clone(),
            owner,
            terminal,
            None,
            Some(delivery),
            generation,
        );
        Ok(json!({"state":"queued","chars":chars,"enter":id}))
    }

    /// Schedules one Enter, refused at once when the terminal no longer
    /// shows `expected` revision or the person typed there recently.
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
        typing_quiet(session)?;
        let (revision, generation) = (session.read_revision(), session.input_generation());
        let id = self.context_admit_enter(owner, terminal)?;
        self.context_schedule_enter(
            id.clone(),
            owner,
            terminal,
            Some(revision),
            None,
            generation,
        );
        Ok(json!({"enter":id,"state":"waiting","delay_ms":wire::ENTER_DELAY_MILLIS}))
    }

    /// Checks one more Enter may wait for `terminal` and allocates its
    /// handle, making room among the finished records.
    fn context_admit_enter(&mut self, owner: u64, terminal: TerminalId) -> Result<String, Error> {
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
        storage::random_token()
            .map_err(|_| Error::new(Code::Unavailable, "Cannot allocate an Enter"))
    }

    pub(super) fn context_schedule_enter(
        &mut self,
        id: String,
        owner: u64,
        terminal: TerminalId,
        revision: Option<u64>,
        after: Option<Delivery>,
        generation: u64,
    ) {
        let now = Instant::now();
        let enter = Enter {
            owner,
            terminal,
            revision,
            after: after.map(|line| (line, now + LINE_DEADLINE)),
            generation,
            attachment: self.app.plugins.attachment_generation,
            due: now + Duration::from_millis(wire::ENTER_DELAY_MILLIS),
            armed: None,
            state: "waiting",
            finished: None,
        };
        self.context.enters.insert(id, enter);
        if let Some(session) = self.app.terminals.get_mut(terminal) {
            session.set_agent_enter_pending(true);
        }
        self.app.plugins.presentation_dirty = true;
    }

    pub(super) fn context_enter_status(&self, owner: u64, enter: &str) -> Result<Value, Error> {
        self.context
            .enters
            .get(enter)
            .filter(|e| e.owner == owner)
            .map(|e| json!({"enter":enter,"state":e.state}))
            .ok_or_else(|| Error::new(Code::NotFound, "Unknown Enter"))
    }

    /// Presses each Enter that is due with nothing changed, and drops every
    /// one whose terminal received other input, whose terminal showed
    /// anything new when it was asked for by revision, whose line was not
    /// written, or whose reader, grant, frontend or terminal went away.
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
            let line = enter.after.clone();
            let allowed = self.context_granted(owner, Scope::TerminalWrite)
                && self.app.plugins.frontend_attached
                && attachment == self.app.plugins.attachment_generation;
            let state = match self.app.terminals.get_mut(terminal) {
                Some(session) if allowed && session.live() => {
                    if revision.is_some_and(|revision| session.read_revision() != revision)
                        || session.input_generation() != generation
                    {
                        "stale"
                    } else if let Some((line, deadline)) = line
                        && line.state() != DeliveryState::Delivered
                    {
                        match line.state() {
                            DeliveryState::Queued | DeliveryState::Writing if now < deadline => {
                                continue;
                            }
                            // Not written in time: a line not yet started is
                            // withdrawn with its Enter.
                            DeliveryState::Queued if line.cancel() => "cancelled",
                            DeliveryState::Cancelled => "cancelled",
                            // Part of the line may be on the screen, and
                            // submitting part of a line is worse than none.
                            _ => "stale",
                        }
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
                if e.delivering() {
                    return DELIVERY_POLL;
                }
                e.armed
                    .map_or(e.due, |armed| armed + ENTER_SETTLE)
                    .saturating_duration_since(now)
            })
            .min()
    }

    /// Moves the deadline of every line still being written into the past.
    #[cfg(test)]
    pub(super) fn context_expire_line_deadlines(&mut self) {
        let past = Instant::now() - Duration::from_millis(1);
        for enter in self.context.enters.values_mut() {
            if let Some((_, deadline)) = enter.after.as_mut() {
                *deadline = past;
            }
        }
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
