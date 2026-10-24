// SPDX-License-Identifier: MPL-2.0

//! The agent channel a workspace host carries: one public conversation and
//! direct messages between participants, kept in memory for as long as the
//! host runs. Participants are found by the connection that joined them;
//! knows nothing about transports, grants or terminals beyond the numbers it
//! is handed.

use crate::plugin::application::{Error, ErrorCode};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, VecDeque};

pub const MAX_PARTICIPANTS: usize = 64;
pub const MAX_MESSAGES: usize = 1_000;
pub const MAX_RETAINED_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_TEXT_BYTES: usize = 16 * 1024;
pub const MAX_READ: usize = 200;
/// Encoded bytes one read returns. An MCP reply carries the data twice, as
/// structured content and as escaped text, so this leaves room under the
/// 2 MiB frame; it is also more than an agent wants to read at once.
pub const MAX_READ_BYTES: usize = 256 * 1024;
const MAX_PROVIDER: usize = 32;
const MAX_TASK: usize = 200;
const MAX_PATH: usize = 4096;
const MAX_CLIENT_FIELD: usize = 64;

/// Where an agent is working, as its MCP process reports it.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Location {
    pub directory: String,
    pub repository: Option<String>,
    pub worktree: Option<String>,
    pub branch: Option<String>,
}

impl Location {
    pub fn validate(&self) -> Result<(), Error> {
        if self.directory.is_empty() {
            return Err(invalid("A location names its directory"));
        }
        for value in [
            Some(&self.directory),
            self.repository.as_ref(),
            self.worktree.as_ref(),
            self.branch.as_ref(),
        ]
        .into_iter()
        .flatten()
        {
            if value.len() > MAX_PATH || value.chars().any(char::is_control) {
                return Err(invalid("Location fields are bounded single-line text"));
            }
        }
        Ok(())
    }

    /// `branch in worktree`, or the directory when there is no repository.
    pub fn summary(&self) -> String {
        match (&self.branch, &self.worktree) {
            (Some(branch), Some(worktree)) => format!("{branch} in {worktree}"),
            (None, Some(worktree)) => worktree.clone(),
            _ => self.directory.clone(),
        }
    }
}

/// The MCP client an agent runs, as its `initialize` request named it.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Client {
    pub name: String,
    pub version: String,
}

#[derive(Clone, Debug)]
pub struct Participant {
    /// A UUID, kept when the same agent joins again.
    pub id: String,
    /// Proves a later join is the same agent. Only the join reply carries it;
    /// the UUID is public, so it cannot serve.
    pub resume: String,
    /// `provider-N`, numbered per provider in the order agents joined.
    pub nick: String,
    /// The MCP identity whose grant admitted it.
    pub identity: String,
    pub client: Option<Client>,
    pub provider: String,
    pub task: String,
    /// The terminal the host verified it runs in.
    pub terminal: Option<u64>,
    pub location: Location,
    pub joined_at_ms: u64,
    pub connection: Option<u64>,
    /// When it disconnected, as a channel revision; the one that left
    /// longest ago gives way when the channel is full.
    pub left: Option<u64>,
    /// The last message it has read.
    pub read_through: u64,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Joined,
    /// Its provider or task changed when it joined again.
    Updated,
    /// It works somewhere else than its earlier messages said.
    Moved,
    Message,
}

#[derive(Clone, Debug)]
pub struct Message {
    pub seq: u64,
    pub at_ms: u64,
    pub kind: Kind,
    pub from: String,
    pub to: Option<String>,
    pub text: String,
    /// Where the sender was when it sent this.
    pub location: Location,
}

impl Message {
    /// Its size as a reply encodes it: the escaped text and location, and
    /// room for the numbers, identities and nicknames around them.
    fn bytes(&self) -> usize {
        let escaped = |text: &str| {
            text.len()
                + text
                    .chars()
                    .map(|c| match c {
                        '"' | '\\' => 1,
                        c if c.is_control() => 5,
                        _ => 0,
                    })
                    .sum::<usize>()
        };
        escaped(&self.text)
            + [
                Some(&self.location.directory),
                self.location.repository.as_ref(),
                self.location.worktree.as_ref(),
                self.location.branch.as_ref(),
            ]
            .into_iter()
            .flatten()
            .map(|field| escaped(field))
            .sum::<usize>()
            + 384
    }
    /// Whether `participant` may read it: everything public, and the direct
    /// messages it sent or received.
    pub fn visible_to(&self, participant: &str) -> bool {
        self.to
            .as_deref()
            .is_none_or(|to| to == participant || self.from == participant)
    }
}

/// What a participant gives when it joins.
pub struct Join<'a> {
    pub identity: &'a str,
    pub connection: u64,
    pub provider: &'a str,
    pub task: &'a str,
    pub client: Option<Client>,
    pub location: Location,
    pub terminal: Option<u64>,
    /// The resume token an earlier join returned.
    pub resume: Option<&'a str>,
    /// The UUID and resume token to use if this is a new participant.
    pub fresh_id: String,
    pub fresh_resume: String,
    pub now_ms: u64,
}

pub struct Read {
    pub messages: Vec<Message>,
    /// The sequence number to read from next.
    pub next: u64,
    /// Whether messages after `since` were dropped before this read.
    pub lost: bool,
}

#[derive(Default)]
pub struct Channel {
    participants: BTreeMap<String, Participant>,
    /// Participant ids in the order they first joined.
    joined_order: Vec<String>,
    messages: VecDeque<Message>,
    retained_bytes: usize,
    last_seq: u64,
    joined_per_provider: BTreeMap<String, u32>,
    /// Nicknames of participants dropped to make room, while retained
    /// messages still name them.
    departed: BTreeMap<String, String>,
    /// Changes whenever anything a reader of the channel could see changes,
    /// participants included.
    revision: u64,
}

impl Channel {
    /// Participants in the order they first joined.
    pub fn participants(&self) -> impl Iterator<Item = &Participant> {
        self.joined_order.iter().map(|id| &self.participants[id])
    }

    pub fn messages(&self) -> impl Iterator<Item = &Message> {
        self.messages.iter()
    }

    pub fn participant(&self, id: &str) -> Option<&Participant> {
        self.participants.get(id)
    }

    pub fn joined(&self, connection: u64) -> Option<&Participant> {
        self.participants
            .values()
            .find(|p| p.connection == Some(connection))
    }

    /// The newest sequence number, which changes whenever anything is added.
    pub fn head(&self) -> u64 {
        self.last_seq
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn nick(&self, id: &str) -> Option<&str> {
        self.participants
            .get(id)
            .map(|p| p.nick.as_str())
            .or_else(|| self.departed.get(id).map(String::as_str))
    }

    pub fn join(&mut self, join: Join<'_>) -> Result<&Participant, Error> {
        let provider = join.provider.trim().to_ascii_lowercase();
        if provider.is_empty()
            || provider.len() > MAX_PROVIDER
            || !provider
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
        {
            return Err(invalid(
                "Name the provider in 1 to 32 letters, digits, dots, dashes or underscores, such as claude or codex",
            ));
        }
        let task = join.task.trim();
        if task.is_empty() || task.chars().count() > MAX_TASK || task.chars().any(char::is_control)
        {
            return Err(invalid(
                "State the main task in one line of 1 to 200 characters",
            ));
        }
        if join.client.as_ref().is_some_and(|client| {
            [&client.name, &client.version]
                .iter()
                .any(|field| field.len() > MAX_CLIENT_FIELD || field.chars().any(char::is_control))
        }) {
            return Err(invalid(
                "Client name and version are bounded single-line text",
            ));
        }
        join.location.validate()?;
        // A connection holds one participant; joining again updates it. An
        // earlier participant comes back only with its private resume token.
        let existing = self
            .joined(join.connection)
            .map(|p| p.id.clone())
            .or_else(|| {
                let resume = join.resume?;
                self.participants
                    .values()
                    .find(|p| p.resume == resume && p.identity == join.identity)
                    .map(|p| p.id.clone())
            });
        self.revision += 1;
        if let Some(id) = existing {
            for participant in self.participants.values_mut() {
                if participant.id != id && participant.connection == Some(join.connection) {
                    participant.connection = None;
                }
            }
            let participant = self
                .participants
                .get_mut(&id)
                .expect("existing participant");
            participant.connection = Some(join.connection);
            participant.left = None;
            participant.client = join.client.or(participant.client.take());
            participant.terminal = join.terminal;
            let changed = participant.provider != provider || participant.task != task;
            participant.provider = provider;
            participant.task = task.to_owned();
            let location = participant.location.clone();
            if changed {
                let text = format!("{}: {}", participant.provider, participant.task);
                self.record(
                    Kind::Updated,
                    &id,
                    None,
                    text,
                    location.clone(),
                    join.now_ms,
                );
            }
            self.relocate(&id, join.location, join.now_ms);
            return Ok(&self.participants[&id]);
        }
        if self.participants.len() >= MAX_PARTICIPANTS && !self.evict_departed() {
            return Err(Error::new(
                ErrorCode::LimitExceeded,
                "The agent channel is full",
            ));
        }
        let count = self
            .joined_per_provider
            .entry(provider.clone())
            .or_default();
        *count += 1;
        let nick = format!("{provider}-{count}");
        let id = join.fresh_id;
        self.joined_order.push(id.clone());
        self.participants.insert(
            id.clone(),
            Participant {
                id: id.clone(),
                resume: join.fresh_resume,
                nick,
                identity: join.identity.to_owned(),
                client: join.client,
                provider,
                task: task.to_owned(),
                terminal: join.terminal,
                location: join.location.clone(),
                joined_at_ms: join.now_ms,
                connection: Some(join.connection),
                left: None,
                read_through: self.last_seq,
            },
        );
        self.record(
            Kind::Joined,
            &id,
            None,
            task.to_owned(),
            join.location,
            join.now_ms,
        );
        Ok(&self.participants[&id])
    }

    /// The participant stays known, so its messages keep their sender and it
    /// can join again; it is no longer reachable through the connection.
    pub fn disconnect(&mut self, connection: u64) {
        for participant in self.participants.values_mut() {
            if participant.connection == Some(connection) {
                participant.connection = None;
                self.revision += 1;
                participant.left = Some(self.revision);
            }
        }
    }

    /// Drops the participant that disconnected longest ago, keeping its
    /// nickname for the messages that name it.
    fn evict_departed(&mut self) -> bool {
        let Some(index) = self
            .joined_order
            .iter()
            .enumerate()
            .filter_map(|(index, id)| self.participants[id].left.map(|left| (left, index)))
            .min()
            .map(|(_, index)| index)
        else {
            return false;
        };
        let id = self.joined_order.remove(index);
        let departed = self.participants.remove(&id).expect("ordered participant");
        self.departed.insert(id, departed.nick);
        let named = |id: &String| {
            self.messages
                .iter()
                .any(|m| &m.from == id || m.to.as_ref() == Some(id))
        };
        let keep = self
            .departed
            .keys()
            .filter(|id| named(id))
            .cloned()
            .collect::<std::collections::BTreeSet<_>>();
        self.departed.retain(|id, _| keep.contains(id));
        true
    }

    pub fn post(
        &mut self,
        connection: u64,
        to: Option<&str>,
        text: &str,
        location: Location,
        now_ms: u64,
    ) -> Result<u64, Error> {
        let from = self.joined(connection).ok_or_else(not_joined)?.id.clone();
        if text.trim().is_empty() {
            return Err(invalid("A message needs text"));
        }
        if text.len() > MAX_TEXT_BYTES {
            return Err(Error::new(
                ErrorCode::LimitExceeded,
                "A message holds at most 16 KiB of text",
            ));
        }
        if text
            .chars()
            .any(|c| c.is_control() && c != '\n' && c != '\t')
        {
            return Err(invalid(
                "A message may not contain control characters other than newline and tab",
            ));
        }
        location.validate()?;
        let to = match to {
            None => None,
            Some(target) => {
                let found = self
                    .participants
                    .values()
                    .find(|p| p.id == target || p.nick == target)
                    .ok_or_else(|| Error::new(ErrorCode::NotFound, "No such participant"))?;
                if found.id == from {
                    return Err(invalid("A direct message goes to someone else"));
                }
                Some(found.id.clone())
            }
        };
        self.relocate(&from, location.clone(), now_ms);
        Ok(self.record(Kind::Message, &from, to, text.to_owned(), location, now_ms))
    }

    pub fn read(&mut self, connection: u64, since: u64, limit: usize) -> Result<Read, Error> {
        let me = self.joined(connection).ok_or_else(not_joined)?.id.clone();
        // A position past the newest message belongs to an earlier host.
        let since = if since > self.last_seq { 0 } else { since };
        let first = self.messages.front().map_or(self.last_seq + 1, |m| m.seq);
        let lost = since.saturating_add(1) < first;
        let mut taken = Vec::new();
        let mut bytes = 0;
        // The last message examined: everything up to it has been either
        // returned or found to be someone else's direct message.
        let mut next = since.max(first - 1);
        for message in self.messages.iter().filter(|m| m.seq > since) {
            let visible = message.visible_to(&me);
            if visible
                && (taken.len() >= limit.clamp(1, MAX_READ)
                    || bytes + message.bytes() > MAX_READ_BYTES)
            {
                break;
            }
            next = message.seq;
            if visible {
                bytes += message.bytes();
                taken.push(message.seq);
            }
        }
        let participant = self.participants.get_mut(&me).expect("joined participant");
        participant.read_through = participant.read_through.max(next);
        Ok(Read {
            messages: self
                .messages
                .iter()
                .filter(|m| taken.binary_search(&m.seq).is_ok())
                .cloned()
                .collect(),
            next,
            lost,
        })
    }

    /// Messages `participant` can read that arrived after its last read and
    /// were not its own.
    pub fn unread(&self, participant: &str) -> usize {
        let Some(read_through) = self.participants.get(participant).map(|p| p.read_through) else {
            return 0;
        };
        self.messages
            .iter()
            .filter(|m| {
                m.seq > read_through
                    && m.from != participant
                    && m.kind == Kind::Message
                    && m.visible_to(participant)
            })
            .count()
    }

    fn relocate(&mut self, id: &str, location: Location, now_ms: u64) {
        let participant = self.participants.get_mut(id).expect("known participant");
        if participant.location == location {
            return;
        }
        let text = format!(
            "moved from {} to {}",
            participant.location.summary(),
            location.summary()
        );
        participant.location = location.clone();
        self.record(Kind::Moved, id, None, text, location, now_ms);
    }

    fn record(
        &mut self,
        kind: Kind,
        from: &str,
        to: Option<String>,
        text: String,
        location: Location,
        at_ms: u64,
    ) -> u64 {
        self.last_seq += 1;
        self.revision += 1;
        let message = Message {
            seq: self.last_seq,
            at_ms,
            kind,
            from: from.to_owned(),
            to,
            text,
            location,
        };
        self.retained_bytes += message.bytes();
        self.messages.push_back(message);
        while self.messages.len() > MAX_MESSAGES || self.retained_bytes > MAX_RETAINED_BYTES {
            let Some(dropped) = self.messages.pop_front() else {
                break;
            };
            self.retained_bytes -= dropped.bytes();
        }
        self.last_seq
    }
}

/// A version 4 UUID spelled from 32 random hexadecimal digits.
pub fn uuid(random_hex: &str) -> String {
    let mut digits: Vec<char> = random_hex.chars().take(32).collect();
    digits.resize(32, '0');
    digits[12] = '4';
    digits[16] = ['8', '9', 'a', 'b'][digits[16].to_digit(16).unwrap_or(0) as usize % 4];
    let digits: String = digits.into_iter().collect();
    format!(
        "{}-{}-{}-{}-{}",
        &digits[..8],
        &digits[8..12],
        &digits[12..16],
        &digits[16..20],
        &digits[20..]
    )
}

fn invalid(message: &str) -> Error {
    Error::new(ErrorCode::InvalidArgument, message)
}

fn not_joined() -> Error {
    Error::new(
        ErrorCode::CapabilityDenied,
        "Join the agent channel first, stating your provider and task",
    )
}

#[cfg(test)]
#[path = "tests/channel.rs"]
mod tests;
