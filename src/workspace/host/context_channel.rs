// SPDX-License-Identifier: MPL-2.0

//! The workspace's agent channel as the context host serves it: requests from
//! readers granted `agent_channel`, the unread marks on the terminals
//! participants run in, and the `[agent channel]` page the person reads.

use super::WorkspaceHost;
use crate::{
    plugin::application::{Error, ErrorCode as Code},
    terminal::TerminalId,
    workspace::context::{
        channel::{self, Join, Kind, Message, Participant},
        storage,
        wire::{ChannelTerminal, Request},
    },
};
use serde_json::{Value, json};

/// Messages the person's page shows, newest last.
const PAGE_MESSAGES: usize = 200;

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
            Request::ChannelPost { text, to, location } => {
                let seq = self.context.channel.post(
                    connection,
                    to.as_deref(),
                    &text,
                    location,
                    now_ms,
                )?;
                Ok(json!({"seq":seq}))
            }
            Request::ChannelRead { since, limit } => {
                let read = self.context.channel.read(connection, since, limit)?;
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
            .filter(|terminal| terminal.live())
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
                "No agent has joined. An agent joins with the join_channel MCP tool once its\nidentity has Read (1 in :mcp).\n",
            );
            return page;
        }
        page.push_str("Participants\n\n");
        for participant in participants {
            let terminal = participant
                .terminal
                .and_then(|id| self.app.terminals.get(TerminalId::from_raw(id)))
                .filter(|terminal| terminal.live())
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
