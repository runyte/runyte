// SPDX-License-Identifier: MPL-2.0

//! The workspace's agent channel as the context host serves it: requests from
//! readers granted `agent_channel`, the unread marks on the terminals
//! participants run in, the nudges senders ask for, and the `[agent channel]`
//! page the person reads.

use super::WorkspaceHost;
use crate::{
    app::context_access::visible,
    notification::{NotificationDraft, NotificationSeverity},
    plugin::application::{Error, ErrorCode as Code},
    terminal::{
        TerminalId,
        proposal::{Delivery, DeliveryState, Text},
    },
    workspace::context::{
        channel::{self, Join, Kind, Message, Participant},
        storage,
        wire::{ChannelTerminal, Request, Scope},
    },
};
use serde_json::{Value, json};
use std::time::{Duration, Instant};

/// Messages the person's page shows, newest last.
const PAGE_MESSAGES: usize = 200;

/// The one line a nudge types. Fixed, so nothing a sender writes reaches a
/// prompt this way; and inert in a shell an agent has exited to: no word of
/// it names a command, even on a filesystem that ignores case, and it has
/// nothing a shell would expand or redirect.
pub(super) const NUDGE_TEXT: &str = "Runyte: you have new messages on the agent channel.";
/// How long after the person last typed in a terminal a nudge still waits.
pub(super) const NUDGE_QUIET: Duration = Duration::from_secs(30);
/// Between a nudge's paste reaching the terminal and its Enter, so the child
/// reads the two apart rather than as one burst it might take for a paste.
const NUDGE_ENTER_DELAY: Duration = Duration::from_millis(250);
/// How often a held nudge looks again at what holds it.
const NUDGE_RECHECK: Duration = Duration::from_secs(1);

/// A nudge a terminal has outstanding. It stays until its participant reads
/// the channel, so a terminal is nudged at most once per read.
pub(super) struct Nudge {
    participant: String,
    /// The sender's nickname, for the notification.
    from: String,
    phase: NudgePhase,
}

enum NudgePhase {
    /// Waiting for the person to stop typing, a draft to be submitted or a
    /// proposal to finish.
    Held,
    /// The line is queued; Enter follows once it is written, if no other
    /// input reached the terminal in between.
    Pasting {
        delivery: Delivery,
        generation: u64,
        written: Option<Instant>,
    },
    Sent,
}

impl WorkspaceHost {
    pub(super) fn context_channel_request(
        &mut self,
        connection: u64,
        identity: &str,
        request: Request,
    ) -> Result<Value, Error> {
        let now_ms = chrono::Utc::now().timestamp_millis().max(0) as u64;
        let result = match request {
            Request::ChannelJoin {
                provider,
                task,
                client,
                location,
                terminal,
                resume,
            } => {
                let terminal = terminal
                    .as_ref()
                    .and_then(|terminal| self.context_channel_terminal(connection, terminal));
                let random = || {
                    storage::random_token()
                        .map_err(|_| Error::new(Code::Unavailable, "Cannot allocate an identity"))
                };
                let fresh_id = channel::uuid(&random()?);
                let fresh_resume = random()?;
                let joined = self
                    .context
                    .channel
                    .join(Join {
                        identity,
                        connection,
                        provider: &provider,
                        task: &task,
                        client,
                        location,
                        terminal,
                        resume: resume.as_deref(),
                        fresh_id,
                        fresh_resume,
                        now_ms,
                    })?
                    .clone();
                // Only this reply carries the resume token.
                Ok(
                    json!({"participant":self.context_participant_json(&joined),"resume":joined.resume,"head":self.context.channel.head()}),
                )
            }
            Request::ChannelPost {
                text,
                to,
                location,
                nudge,
            } => {
                let posted = self.context.channel.post(
                    connection,
                    to.as_deref(),
                    &text,
                    location,
                    nudge.as_deref().unwrap_or_default(),
                    now_ms,
                )?;
                if nudge.is_none() {
                    return self.context_channel_posted(json!({"seq":posted.seq}));
                }
                let from = self
                    .context
                    .channel
                    .joined(connection)
                    .map(|p| p.nick.clone())
                    .unwrap_or_default();
                let nudges = posted
                    .nudge
                    .iter()
                    .map(|participant| {
                        let state = self.context_nudge(&from, participant);
                        json!({"participant":participant,"nick":self.context.channel.nick(participant),"state":state})
                    })
                    .collect::<Vec<_>>();
                Ok(json!({"seq":posted.seq,"nudges":nudges}))
            }
            Request::ChannelRead { since, limit } => {
                let read = self.context.channel.read(connection, since, limit)?;
                // Having read, the participant needs no nudge for what came
                // before; one already typing finishes, so its line is not
                // left on the prompt without its Enter.
                if let Some(me) = self
                    .context
                    .channel
                    .joined(connection)
                    .map(|p| p.id.clone())
                {
                    self.context.nudges.retain(|_, nudge| {
                        nudge.participant != me || matches!(nudge.phase, NudgePhase::Pasting { .. })
                    });
                }
                let messages = read
                    .messages
                    .iter()
                    .map(|message| self.context_message_json(message))
                    .collect::<Vec<_>>();
                Ok(
                    json!({"messages":messages,"next":read.next,"lost":read.lost,"head":self.context.channel.head()}),
                )
            }
            Request::ChannelWho(_) => {
                let participants = self
                    .context
                    .channel
                    .participants()
                    .map(|participant| self.context_participant_json(participant))
                    .collect::<Vec<_>>();
                Ok(json!({"participants":participants}))
            }
            _ => Err(Error::new(Code::Internal, "Not a channel request")),
        };
        self.context_channel_changed();
        result
    }

    fn context_channel_posted(&mut self, value: Value) -> Result<Value, Error> {
        self.context_channel_changed();
        Ok(value)
    }

    /// Asks for a nudge of `participant` and says what became of it: `sent`
    /// once its line is queued, `held` while the terminal is not ready,
    /// `already_pending` when the terminal has one outstanding, and
    /// `unavailable` for every reason it cannot be nudged at all, which the
    /// sender is not told apart.
    fn context_nudge(&mut self, from: &str, participant: &str) -> &'static str {
        // Drops outstanding nudges whose participant can no longer be reached.
        self.context_channel_nudges();
        let Some(terminal) = self.context_nudge_target(participant) else {
            return "unavailable";
        };
        if !self
            .app
            .terminals
            .get(TerminalId::from_raw(terminal))
            .is_some_and(|t| t.proposal_input_signature().modes.bracketed_paste)
        {
            return "unavailable";
        }
        if self.context.nudges.contains_key(&terminal) {
            return "already_pending";
        }
        self.context.nudges.insert(
            terminal,
            Nudge {
                participant: participant.to_owned(),
                from: from.to_owned(),
                phase: NudgePhase::Held,
            },
        );
        self.context_channel_nudges();
        match self.context.nudges.get(&terminal).map(|n| &n.phase) {
            Some(NudgePhase::Held) => "held",
            Some(_) => "sent",
            // Refused by the terminal as it was typed.
            None => "unavailable",
        }
    }

    /// The terminal `participant` may be nudged in: it is connected, its
    /// identity's grant here accepts nudges, and the host verified the live
    /// terminal it runs in.
    fn context_nudge_target(&self, participant: &str) -> Option<u64> {
        let participant = self.context.channel.participant(participant)?;
        let terminal = participant.terminal?;
        (self.context_granted(participant.connection?, Scope::ChannelNudge)
            && self
                .app
                .terminals
                .get(TerminalId::from_raw(terminal))
                .is_some_and(|t| t.live()))
        .then_some(terminal)
    }

    /// Moves each outstanding nudge on: types a held one once nothing holds
    /// it, presses Enter after a written paste, and forgets any whose
    /// participant or terminal has gone or whose grant was withdrawn.
    pub(super) fn context_channel_nudges(&mut self) {
        let now = Instant::now();
        let terminals = self.context.nudges.keys().copied().collect::<Vec<_>>();
        for terminal in terminals {
            let nudge = self.context.nudges.remove(&terminal).expect("listed nudge");
            if self.context_nudge_target(&nudge.participant) != Some(terminal) {
                if let NudgePhase::Pasting { delivery, .. } = &nudge.phase {
                    delivery.cancel();
                }
                continue;
            }
            let id = TerminalId::from_raw(terminal);
            let proposed = self.context_terminal_proposed(id);
            let Some(session) = self.app.terminals.get_mut(id) else {
                continue;
            };
            let phase = match nudge.phase {
                NudgePhase::Held => {
                    let typing = session
                        .typed_at()
                        .is_some_and(|typed| now.duration_since(typed) < NUDGE_QUIET);
                    if typing
                        || session.has_draft()
                        || proposed
                        || !session.proposal_input_signature().modes.bracketed_paste
                    {
                        NudgePhase::Held
                    } else {
                        let text = Text::new(NUDGE_TEXT).expect("valid nudge text");
                        match session.paste_nudge(&text) {
                            Ok((delivery, generation)) => NudgePhase::Pasting {
                                delivery,
                                generation,
                                written: None,
                            },
                            Err(_) => continue,
                        }
                    }
                }
                NudgePhase::Pasting {
                    delivery,
                    generation,
                    written,
                } => match (delivery.state(), written) {
                    (DeliveryState::Queued | DeliveryState::Writing, _) => NudgePhase::Pasting {
                        delivery,
                        generation,
                        written,
                    },
                    (DeliveryState::Delivered, None) => NudgePhase::Pasting {
                        delivery,
                        generation,
                        written: Some(now),
                    },
                    (DeliveryState::Delivered, Some(at))
                        if now.duration_since(at) < NUDGE_ENTER_DELAY =>
                    {
                        NudgePhase::Pasting {
                            delivery,
                            generation,
                            written,
                        }
                    }
                    (DeliveryState::Delivered, Some(_)) => {
                        if !session.submit_nudge(generation) {
                            continue;
                        }
                        let target = match session.number() {
                            Some(number) => format!("terminal #{number}"),
                            None => "a terminal".to_owned(),
                        };
                        let to = self
                            .context
                            .channel
                            .nick(&nudge.participant)
                            .unwrap_or("?")
                            .to_owned();
                        // Enter reached an agent's prompt on someone else's
                        // request; say so where it can be traced afterwards.
                        self.app.push_notification(NotificationDraft::new(
                            NotificationSeverity::Info,
                            "Agent channel",
                            "Channel nudge sent",
                            format!(
                                "{} nudged {} in {target} to read the agent channel.",
                                visible(&nudge.from),
                                visible(&to),
                            ),
                        ));
                        NudgePhase::Sent
                    }
                    // Typing cancelled it, or its outcome is unknown.
                    _ => continue,
                },
                NudgePhase::Sent => NudgePhase::Sent,
            };
            self.context
                .nudges
                .insert(terminal, Nudge { phase, ..nudge });
        }
    }

    /// When outstanding nudges next need looking at.
    pub(super) fn context_nudge_delay(&self) -> Option<Duration> {
        self.context
            .nudges
            .values()
            .filter_map(|nudge| match nudge.phase {
                NudgePhase::Held => Some(NUDGE_RECHECK),
                NudgePhase::Pasting { .. } => Some(Duration::from_millis(50)),
                NudgePhase::Sent => None,
            })
            .min()
    }

    /// The terminal a joining agent runs in, if Runyte handed that terminal's
    /// children the capability it presents and the connecting process belongs
    /// to that terminal's session. Anything less leaves the terminal unknown.
    fn context_channel_terminal(&self, connection: u64, terminal: &ChannelTerminal) -> Option<u64> {
        #[cfg(unix)]
        {
            let peer = self.context.peers.get(&connection).copied().flatten();
            self.app
                .terminals
                .validates_parent(
                    TerminalId::from_raw(terminal.terminal),
                    &terminal.capability,
                    peer,
                )
                .then_some(terminal.terminal)
        }
        #[cfg(not(unix))]
        {
            let _ = (connection, terminal);
            None
        }
    }

    fn context_participant_json(&self, participant: &Participant) -> Value {
        let terminal = participant
            .terminal
            .and_then(|id| self.app.terminals.get(TerminalId::from_raw(id)))
            .map(|terminal| {
                json!({"number":terminal.number(),"title":crate::snapshot::terminal_label(terminal)})
            });
        json!({
            "participant":participant.id,
            "nick":participant.nick,
            "identity":participant.identity,
            "client":participant.client,
            "provider":participant.provider,
            "task":participant.task,
            "terminal":terminal,
            "location":participant.location,
            "joined_at_ms":participant.joined_at_ms,
            "connected":participant.connection.is_some(),
        })
    }

    fn context_message_json(&self, message: &Message) -> Value {
        let channel = &self.context.channel;
        json!({
            "seq":message.seq,
            "at_ms":message.at_ms,
            "kind":message.kind,
            "from":message.from,
            "from_nick":channel.nick(&message.from),
            "to":message.to,
            "to_nick":message.to.as_deref().and_then(|to| channel.nick(to)),
            "text":message.text,
            "location":message.location,
        })
    }

    /// Brings what the person sees up to date: each terminal running a
    /// participant shows how many messages it has not read, and an open
    /// `[agent channel]` page shows the conversation as it now stands.
    pub(super) fn context_channel_changed(&mut self) {
        let mut unread = std::collections::BTreeMap::<u64, usize>::new();
        for participant in self.context.channel.participants() {
            if let Some(terminal) = participant.terminal
                && participant.connection.is_some()
            {
                *unread.entry(terminal).or_default() +=
                    self.context.channel.unread(&participant.id);
            }
        }
        for id in self.app.terminals.ids() {
            let count = unread.get(&id.get()).copied().unwrap_or_default();
            if let Some(terminal) = self.app.terminals.get_mut(id)
                && terminal.channel_unread() != count
            {
                terminal.set_channel_unread(count);
                self.app.plugins.presentation_dirty = true;
            }
        }
        // Rendering is the expensive part, so only an open page is redrawn.
        if self.context.channel.revision() != self.context.channel_page_revision
            && self.app.agent_channel_page_open()
        {
            self.context.channel_page_revision = self.context.channel.revision();
            let page = self.context_channel_page();
            self.app.refresh_agent_channel_page(&page);
        }
    }

    pub(super) fn context_channel_page(&self) -> String {
        let channel = &self.context.channel;
        let participants = channel.participants().collect::<Vec<_>>();
        let mut page = format!(
            "Agent channel · {} participant{} · messages last as long as this workspace host\n\n",
            participants.len(),
            if participants.len() == 1 { "" } else { "s" }
        );
        if participants.is_empty() {
            page.push_str(
                "No agent has joined. An agent joins with the join_channel MCP tool once its\nidentity has the agent channel permission (6 in :mcp).\n",
            );
            return page;
        }
        page.push_str("Participants\n\n");
        for participant in participants {
            let terminal = participant
                .terminal
                .and_then(|id| self.app.terminals.get(TerminalId::from_raw(id)))
                .map_or_else(
                    || "terminal unknown".to_owned(),
                    crate::snapshot::terminal_label,
                );
            page.push_str(&format!(
                "  {}  {} · {}\n    {} · {} · {}{}\n    {}\n",
                participant.nick,
                participant.provider,
                participant.task,
                terminal,
                participant.location.summary(),
                participant.identity,
                participant
                    .client
                    .as_ref()
                    .map(|client| format!(" · {} {}", client.name, client.version))
                    .unwrap_or_default(),
                if participant.connection.is_some() {
                    participant.id.clone()
                } else {
                    format!("{} · disconnected", participant.id)
                },
            ));
        }
        page.push_str("\nMessages\n\n");
        let messages = channel.messages().collect::<Vec<_>>();
        let skipped = messages.len().saturating_sub(PAGE_MESSAGES);
        if skipped > 0 {
            page.push_str(&format!("  … {skipped} earlier messages\n"));
        }
        for message in &messages[skipped..] {
            let at = chrono::DateTime::from_timestamp_millis(message.at_ms as i64)
                .map(|at| {
                    at.with_timezone(&chrono::Local)
                        .format("%H:%M:%S")
                        .to_string()
                })
                .unwrap_or_default();
            let from = channel.nick(&message.from).unwrap_or("?");
            let line = match message.kind {
                Kind::Message => match message.to.as_deref() {
                    Some(to) => format!(
                        "{from} → {} (direct): {}",
                        channel.nick(to).unwrap_or("?"),
                        message.text
                    ),
                    None => format!("{from}: {}", message.text),
                },
                Kind::Joined => format!("{from} joined: {}", message.text),
                Kind::Updated => format!("{from} now {}", message.text),
                Kind::Moved => format!("{from} {}", message.text),
            };
            let mut lines = line.lines();
            page.push_str(&format!("  {at}  {}\n", lines.next().unwrap_or_default()));
            for rest in lines {
                page.push_str(&format!("            {rest}\n"));
            }
        }
        page
    }
}
