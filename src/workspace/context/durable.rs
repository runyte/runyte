// SPDX-License-Identifier: MPL-2.0

//! Transactional storage foundation for the opt-in durable channel protocol.
//! Not served by the legacy channel. Callers must run this blocking API on a
//! bounded worker and supply a currently authorized host route on every call.
//! Native policy methods are deliberately separate from participant operations.

use super::{channel::Location, storage::random_token};
use crate::{hash::sha256_hex, private_storage::Directory};
use anyhow::{Context, Result, ensure};
use rusqlite::{Connection, OpenFlags, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use std::{ffi::OsStr, path::Path, time::Duration};

const FILE: &str = "agent-channels.sqlite3";
const SCHEMA: i64 = 1;
const APPLICATION: i64 = 0x52594348;
pub const RETRY_MS: u64 = 7 * 24 * 60 * 60 * 1000;
pub const RECEIPT_MS: u64 = 24 * 60 * 60 * 1000;
const MAX_RECEIPTS: i64 = 64;
const MAX_SENDS: i64 = 8192;
const MAX_MEMBERS: usize = 64;
const MAX_CHANNELS: i64 = 256;
const MAX_PARTICIPANTS: i64 = 4096;

/// An exact host workspace identity and granted MCP identity. This is not an
/// authority token: the host must check its live native grant before dispatch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Route {
    pub workspace: String,
    pub identity: String,
}
impl Route {
    fn validate(&self) -> Result<()> {
        single_line(&self.workspace, 256)?;
        single_line(&self.identity, 64)
    }
}

/// Does not carry a recovery secret. Every operation revalidates its binding.
#[derive(Clone, Debug)]
pub struct Actor {
    id: String,
    route: Route,
}
impl Actor {
    pub fn id(&self) -> &str {
        &self.id
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConversationKind {
    Public,
    Fixed,
}
impl ConversationKind {
    fn name(self) -> &'static str {
        match self {
            Self::Public => "public",
            Self::Fixed => "fixed",
        }
    }
}

/// A caller-generated nonce with an immutable creation time. Expired keys are
/// rejected even after their deduplication tombstone has been collected.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SendKey {
    pub issued_ms: u64,
    pub nonce: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct StoredMessage {
    pub seq: u64,
    pub sender: String,
    pub text: String,
    pub location: Location,
    pub at_ms: u64,
}
#[derive(Debug)]
pub struct Page {
    pub messages: Vec<StoredMessage>,
    pub receipt: Option<String>,
    /// Pass the first sequence as `before` to fetch the preceding history page.
    pub older_before: Option<u64>,
    pub more: bool,
}
#[derive(Clone, Copy)]
pub enum ReadMode {
    Unread,
    History { before: Option<u64> },
}

/// One blocking worker's database connection. No production startup opens it.
/// The root is explicit so tests never select the operator's home implicitly.
pub struct Store {
    db: Connection,
    _directory: Directory,
}
impl Store {
    pub fn open(root: &Path) -> Result<Self> {
        ensure!(root.is_absolute(), "channel storage root must be absolute");
        let parent = root
            .parent()
            .context("channel storage needs an existing parent")?;
        #[cfg(unix)]
        let directory = {
            let anchor = Directory::open_existing(parent, false)?;
            let directory = Directory::open(root, true)?;
            anchor.sync()?;
            directory
        };
        #[cfg(windows)]
        let directory = Directory::open_durable_beneath(
            parent,
            Path::new(root.file_name().context("invalid storage root")?),
        )?;
        // Precreate the database privately. Admit existing SQLite sidecars too;
        // no file may be a symlink, hard link, special file or foreign-owned file.
        for name in [
            FILE,
            "agent-channels.sqlite3-wal",
            "agent-channels.sqlite3-shm",
            "agent-channels.sqlite3-journal",
        ] {
            match directory.open_read(OsStr::new(name)) {
                Ok(file) => {
                    #[cfg(windows)]
                    drop(file);
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::PermissionsExt;
                        ensure!(
                            file.metadata()?.permissions().mode() & 0o077 == 0,
                            "channel storage must be owner-private"
                        );
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    if name == FILE {
                        match directory.create_new(OsStr::new(name)) {
                            Ok(_) => (),
                            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                                // A concurrent opener won. Re-admit it through
                                // the same ownership and no-link boundary.
                                let file = directory.open_read(OsStr::new(name))?;
                                #[cfg(windows)]
                                drop(file);
                                #[cfg(unix)]
                                {
                                    use std::os::unix::fs::PermissionsExt;
                                    ensure!(
                                        file.metadata()?.permissions().mode() & 0o077 == 0,
                                        "channel storage must be owner-private"
                                    );
                                }
                            }
                            Err(e) => return Err(e.into()),
                        }
                    }
                }
                Err(e) => return Err(e.into()),
            }
        }
        directory.sync()?;
        let mut db = Connection::open_with_flags(
            root.join(FILE),
            OpenFlags::SQLITE_OPEN_READ_WRITE
                | OpenFlags::SQLITE_OPEN_NO_MUTEX
                | OpenFlags::SQLITE_OPEN_NOFOLLOW,
        )?;
        db.busy_timeout(Duration::from_millis(100))?;
        db.pragma_update(None, "foreign_keys", true)?;
        db.pragma_update(None, "synchronous", "FULL")?;
        let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let version: i64 = tx.pragma_query_value(None, "user_version", |r| r.get(0))?;
        let application: i64 = tx.pragma_query_value(None, "application_id", |r| r.get(0))?;
        ensure!(
            (version == SCHEMA && application == APPLICATION) || (version == 0 && application == 0),
            "unsupported channel database schema"
        );
        if version == 0 {
            let tables: i64 = tx.query_row(
                "SELECT count(*) FROM sqlite_schema WHERE name NOT LIKE 'sqlite_%'",
                [],
                |r| r.get(0),
            )?;
            ensure!(tables == 0, "refusing to replace an unknown database");
            tx.execute_batch(include_str!("durable/schema.sql"))?;
            tx.pragma_update(None, "application_id", APPLICATION)?;
        }
        tx.commit()?;
        db.pragma_update(None, "journal_mode", "WAL")?;
        db.pragma_update(None, "synchronous", "FULL")?;
        // A finite ceiling rejects writes rather than evicting history. This
        // initial 256 MiB limit covers pages, not transient WAL or filesystem use.
        db.pragma_update(None, "max_page_count", 65536)?;
        Ok(Self {
            db,
            _directory: directory,
        })
    }

    /// A first registration is explicit; a recovery attempt never creates a
    /// replacement identity. A stable credential is persisted by the adapter
    /// before first registration, so a lost registration reply is retryable.
    pub fn register(
        &mut self,
        route: &Route,
        secret: &str,
        provider: &str,
        task: &str,
    ) -> Result<Actor> {
        route.validate()?;
        credential(secret)?;
        single_line(provider, 32)?;
        single_line(task, 800)?;
        let hash = sha256_hex(secret.as_bytes());
        let tx = self
            .db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        if let Some(id) = tx
            .query_row(
                "SELECT id FROM participants WHERE secret_hash=?1",
                [&hash],
                |r| r.get::<_, String>(0),
            )
            .optional()?
        {
            let actor = Actor {
                id,
                route: route.clone(),
            };
            binding(&tx, &actor)?;
            return Ok(actor);
        }
        let count: i64 = tx.query_row("SELECT count(*) FROM participants", [], |r| r.get(0))?;
        ensure!(
            count < MAX_PARTICIPANTS,
            "participant storage limit reached"
        );
        let id = super::channel::uuid(&random_token()?);
        tx.execute(
            "INSERT INTO participants VALUES (?1,?2,?3,?4)",
            params![id, hash, provider, task],
        )?;
        tx.execute(
            "INSERT INTO bindings VALUES (?1,?2,?3)",
            params![id, route.workspace, route.identity],
        )?;
        tx.commit()?;
        Ok(Actor {
            id,
            route: route.clone(),
        })
    }

    pub fn resume(&self, route: &Route, secret: &str) -> Result<Actor> {
        route.validate()?;
        credential(secret)?;
        let id = self
            .db
            .query_row(
                "SELECT id FROM participants WHERE secret_hash=?1",
                [sha256_hex(secret.as_bytes())],
                |r| r.get(0),
            )
            .optional()?
            .context("participant recovery denied")?;
        let actor = Actor {
            id,
            route: route.clone(),
        };
        binding(&self.db, &actor)?;
        Ok(actor)
    }

    /// Native operator operation; never expose this as an agent tool. Current
    /// grants are still required, and binding alone grants no channel access.
    pub fn bind_route(&mut self, participant: &str, route: &Route) -> Result<()> {
        route.validate()?;
        self.db.execute(
            "INSERT OR IGNORE INTO bindings VALUES (?1,?2,?3)",
            params![participant, route.workspace, route.identity],
        )?;
        Ok(())
    }

    /// Native operator policy replacement with compare-and-swap revision.
    /// No agent operation can add another workspace or remove a policy fence.
    pub fn replace_policy(
        &mut self,
        channel: &str,
        expected_revision: u64,
        routes: &[Route],
    ) -> Result<u64> {
        ensure!(routes.len() <= MAX_MEMBERS, "too many channel routes");
        for route in routes {
            route.validate()?;
        }
        let tx = self
            .db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let changed = tx.execute(
            "UPDATE channels SET revision=revision+1 WHERE id=?1 AND revision=?2 AND retired=0",
            params![channel, number(expected_revision)?],
        )?;
        ensure!(changed == 1, "channel policy changed or is unavailable");
        tx.execute("DELETE FROM policy WHERE channel=?1", [channel])?;
        for route in routes {
            tx.execute(
                "INSERT OR IGNORE INTO policy VALUES (?1,?2,?3)",
                params![channel, route.workspace, route.identity],
            )?;
        }
        tx.commit()?;
        Ok(expected_revision + 1)
    }

    /// Creates a local channel. Other granted identities in the same workspace
    /// can join a public channel. Fixed membership is immutable, includes the
    /// creator, and cannot name a participant outside the current route.
    pub fn create(
        &mut self,
        actor: &Actor,
        name: &str,
        kind: ConversationKind,
        members: &[String],
    ) -> Result<String> {
        single_line(name, 128)?;
        ensure!(members.len() < MAX_MEMBERS, "too many fixed members");
        ensure!(
            kind == ConversationKind::Fixed || members.is_empty(),
            "public channels have subscribers, not fixed members"
        );
        let tx = self
            .db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        binding(&tx, actor)?;
        let count: i64 = tx.query_row("SELECT count(*) FROM channels", [], |r| r.get(0))?;
        ensure!(count < MAX_CHANNELS, "channel storage limit reached");
        let id = super::channel::uuid(&random_token()?);
        tx.execute(
            "INSERT INTO channels(id,name,kind) VALUES (?1,?2,?3)",
            params![id, name, kind.name()],
        )?;
        tx.execute(
            "INSERT INTO policy VALUES (?1,?2,'*')",
            params![id, actor.route.workspace],
        )?;
        tx.execute("INSERT INTO members VALUES (?1,?2)", params![id, actor.id])?;
        for member in members {
            let known: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM bindings WHERE participant=?1 AND workspace=?2)",
                params![member, actor.route.workspace],
                |r| r.get(0),
            )?;
            ensure!(known, "group participant unavailable");
            tx.execute(
                "INSERT OR IGNORE INTO members VALUES (?1,?2)",
                params![id, member],
            )?;
        }
        if kind == ConversationKind::Fixed {
            let count: i64 = tx.query_row(
                "SELECT count(*) FROM members WHERE channel=?1",
                [&id],
                |r| r.get(0),
            )?;
            ensure!(
                count >= 2,
                "a fixed group needs at least two distinct participants"
            );
        }
        tx.commit()?;
        Ok(id)
    }

    /// First subscription captures its latest-20 initial view and all later
    /// sends add pending rows. A replay never silently jumps to a newer page.
    pub fn subscribe(&mut self, actor: &Actor, channel: &str) -> Result<()> {
        let tx = self
            .db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let kind = authorized(&tx, actor, channel, false)?;
        if kind == "fixed" {
            member(&tx, actor, channel)?;
            return Ok(());
        }
        let exists: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM members WHERE channel=?1 AND participant=?2)",
            params![channel, actor.id],
            |r| r.get(0),
        )?;
        if !exists {
            let count: i64 = tx.query_row(
                "SELECT count(*) FROM members WHERE channel=?1",
                [channel],
                |r| r.get(0),
            )?;
            ensure!(
                count < MAX_MEMBERS as i64,
                "channel membership limit reached"
            );
            tx.execute(
                "INSERT INTO members VALUES (?1,?2)",
                params![channel, actor.id],
            )?;
            tx.execute("INSERT INTO pending SELECT ?2,?1,seq FROM (SELECT seq,sender FROM messages WHERE channel=?2 ORDER BY seq DESC LIMIT 20) WHERE sender<>?1", params![actor.id, channel])?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn post(
        &mut self,
        actor: &Actor,
        channel: &str,
        key: &SendKey,
        text: &str,
        location: &Location,
        now_ms: u64,
    ) -> Result<u64> {
        credential(&key.nonce)?;
        ensure!(
            key.issued_ms <= now_ms && now_ms - key.issued_ms < RETRY_MS,
            "send key expired or is from the future"
        );
        ensure!(
            !text.trim().is_empty()
                && text.len() <= super::channel::MAX_TEXT_BYTES
                && !text
                    .chars()
                    .any(|c| c.is_control() && c != '\n' && c != '\t'),
            "invalid message text"
        );
        location
            .validate()
            .map_err(|_| anyhow::anyhow!("invalid message location"))?;
        self.cleanup(now_ms)?;
        let key_text = serde_json::to_string(key)?;
        let location_text = serde_json::to_string(location)?;
        let digest = sha256_hex(serde_json::to_string(&(text, location))?.as_bytes());
        let tx = self
            .db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        authorized(&tx, actor, channel, true)?;
        let earlier = tx
            .query_row(
                "SELECT digest,seq FROM sends WHERE participant=?1 AND channel=?2 AND key=?3",
                params![actor.id, channel, key_text],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)),
            )
            .optional()?;
        if let Some((prior, seq)) = earlier {
            ensure!(prior == digest, "send key reused with different content");
            return Ok(seq as u64);
        }
        let count: i64 = tx.query_row(
            "SELECT count(*) FROM sends WHERE participant=?1",
            [&actor.id],
            |r| r.get(0),
        )?;
        ensure!(count < MAX_SENDS, "send retry storage limit reached");
        tx.execute(
            "INSERT INTO messages(channel,sender,text,location,at_ms) VALUES (?1,?2,?3,?4,?5)",
            params![channel, actor.id, text, location_text, number(now_ms)?],
        )?;
        let seq = tx.last_insert_rowid();
        tx.execute("INSERT INTO pending SELECT ?2,participant,?1 FROM members WHERE channel=?2 AND participant<>?3", params![seq, channel, actor.id])?;
        tx.execute(
            "INSERT INTO sends VALUES (?1,?2,?3,?4,?5,?6)",
            params![
                actor.id,
                channel,
                key_text,
                digest,
                seq,
                number(
                    key.issued_ms
                        .checked_add(RETRY_MS)
                        .context("invalid send time")?
                )?
            ],
        )?;
        tx.commit()?;
        Ok(seq as u64)
    }

    /// Fetching never acknowledges. Receipts authorize exactly the returned
    /// IDs, including for history pages, with no blanket cursor advancement.
    pub fn read(
        &mut self,
        actor: &Actor,
        channel: &str,
        mode: ReadMode,
        limit: usize,
        now_ms: u64,
    ) -> Result<Page> {
        ensure!(
            (1..=super::channel::MAX_READ).contains(&limit),
            "invalid read limit"
        );
        self.cleanup(now_ms)?;
        let tx = self
            .db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        authorized(&tx, actor, channel, true)?;
        let (query, bound) = match mode {
            ReadMode::Unread => (
                "SELECT m.seq,m.sender,m.text,m.location,m.at_ms FROM pending p JOIN messages m ON m.seq=p.seq WHERE p.participant=?1 AND p.channel=?2 AND p.seq<?3 ORDER BY p.seq LIMIT ?4",
                i64::MAX,
            ),
            ReadMode::History { before } => (
                "SELECT seq,sender,text,location,at_ms FROM messages WHERE ?1 IS NOT NULL AND channel=?2 AND seq<?3 ORDER BY seq DESC LIMIT ?4",
                before.map(number).transpose()?.unwrap_or(i64::MAX),
            ),
        };
        let mut statement = tx.prepare(query)?;
        let mut rows = statement.query(params![actor.id, channel, bound, limit as i64 + 1])?;
        let mut messages = Vec::new();
        let mut bytes = 0;
        let mut more = false;
        while let Some(row) = rows.next()? {
            let location: String = row.get(3)?;
            let message = StoredMessage {
                seq: row.get::<_, i64>(0)? as u64,
                sender: row.get(1)?,
                text: row.get(2)?,
                location: serde_json::from_str(&location)?,
                at_ms: row.get::<_, i64>(4)? as u64,
            };
            let size = serde_json::to_vec(&message)?.len();
            if messages.len() >= limit || bytes + size > super::channel::MAX_READ_BYTES {
                more = true;
                break;
            }
            bytes += size;
            messages.push(message);
        }
        drop(rows);
        drop(statement);
        messages.sort_by_key(|m| m.seq);
        let older_before = if let Some(first) = messages.first() {
            let exists: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM messages WHERE channel=?1 AND seq<?2)",
                params![channel, number(first.seq)?],
                |r| r.get(0),
            )?;
            exists.then_some(first.seq)
        } else {
            None
        };
        let receipt = if messages.is_empty() {
            None
        } else {
            let ids: Vec<_> = messages.iter().map(|m| m.seq).collect();
            let digest = sha256_hex(serde_json::to_string(&ids)?.as_bytes());
            tx.execute("DELETE FROM receipts WHERE participant=?1 AND channel=?2 AND digest=?3 AND expires<=?4", params![actor.id, channel, digest, number(now_ms)?])?;
            let prior: Option<String> = tx
                .query_row(
                    "SELECT id FROM receipts WHERE participant=?1 AND channel=?2 AND digest=?3",
                    params![actor.id, channel, digest],
                    |r| r.get(0),
                )
                .optional()?;
            Some(if let Some(id) = prior {
                id
            } else {
                let count: i64 = tx.query_row(
                    "SELECT count(*) FROM receipts WHERE participant=?1 AND acknowledged=0",
                    [&actor.id],
                    |r| r.get(0),
                )?;
                ensure!(
                    count < MAX_RECEIPTS,
                    "acknowledge outstanding message batches before reading more"
                );
                let total: i64 = tx.query_row(
                    "SELECT count(*) FROM receipts WHERE participant=?1",
                    [&actor.id],
                    |r| r.get(0),
                )?;
                ensure!(
                    total < MAX_SENDS,
                    "receipt retry storage limit reached; wait for expiry"
                );
                let id = random_token()?;
                tx.execute("INSERT INTO receipts(id,participant,channel,digest,expires) VALUES (?1,?2,?3,?4,?5)", params![id, actor.id, channel, digest, number(now_ms.checked_add(RECEIPT_MS).context("invalid receipt time")?)?])?;
                for seq in ids {
                    tx.execute(
                        "INSERT INTO receipt_items VALUES (?1,?2)",
                        params![id, number(seq)?],
                    )?;
                }
                id
            })
        };
        tx.commit()?;
        Ok(Page {
            messages,
            receipt,
            older_before,
            more,
        })
    }

    pub fn acknowledge(
        &mut self,
        actor: &Actor,
        channel: &str,
        receipt: &str,
        now_ms: u64,
    ) -> Result<()> {
        let tx = self
            .db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let kind = authorized(&tx, actor, channel, true)?;
        let valid: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM receipts WHERE id=?1 AND participant=?2 AND channel=?3 AND expires>?4)", params![receipt, actor.id, channel, number(now_ms)?], |r| r.get(0))?;
        ensure!(
            valid,
            "receipt unavailable or expired; read the outstanding messages again"
        );
        tx.execute("DELETE FROM pending WHERE participant=?1 AND channel=?3 AND seq IN (SELECT seq FROM receipt_items WHERE receipt=?2)", params![actor.id, receipt, channel])?;
        tx.execute("UPDATE receipts SET acknowledged=1 WHERE id=?1", [receipt])?;
        if kind == "fixed" {
            tx.execute("DELETE FROM messages WHERE channel=?1 AND seq IN (SELECT seq FROM receipt_items WHERE receipt=?2) AND NOT EXISTS (SELECT 1 FROM pending WHERE pending.seq=messages.seq)", params![channel, receipt])?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Explicit native operator retirement; never inferred from disconnection.
    pub fn retire(&mut self, channel: &str, expected_revision: u64) -> Result<()> {
        let tx = self
            .db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        ensure!(tx.execute("UPDATE channels SET retired=1,revision=revision+1 WHERE id=?1 AND revision=?2 AND retired=0", params![channel, number(expected_revision)?])? == 1, "channel changed or unavailable");
        tx.execute("DELETE FROM messages WHERE channel=?1", [channel])?;
        tx.execute("DELETE FROM receipts WHERE channel=?1", [channel])?;
        tx.commit()?;
        Ok(())
    }
}

fn single_line(value: &str, max: usize) -> Result<()> {
    ensure!(
        !value.trim().is_empty() && value.len() <= max && !value.chars().any(char::is_control),
        "invalid bounded text"
    );
    Ok(())
}
fn credential(value: &str) -> Result<()> {
    ensure!(
        value.len() == 64 && value.bytes().all(|b| b.is_ascii_hexdigit()),
        "invalid credential or nonce"
    );
    Ok(())
}
fn number(value: u64) -> Result<i64> {
    Ok(i64::try_from(value)?)
}
fn binding(db: &Connection, actor: &Actor) -> Result<()> {
    let found: bool = db.query_row("SELECT EXISTS(SELECT 1 FROM bindings WHERE participant=?1 AND workspace=?2 AND identity=?3)", params![actor.id, actor.route.workspace, actor.route.identity], |r| r.get(0))?;
    ensure!(found, "participant recovery denied");
    Ok(())
}
fn member(db: &Connection, actor: &Actor, channel: &str) -> Result<()> {
    let found: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM members WHERE channel=?1 AND participant=?2)",
        params![channel, actor.id],
        |r| r.get(0),
    )?;
    ensure!(found, "channel unavailable");
    Ok(())
}
fn authorized(db: &Connection, actor: &Actor, channel: &str, membership: bool) -> Result<String> {
    binding(db, actor)?;
    let kind: Option<String> = db.query_row("SELECT c.kind FROM channels c JOIN policy p ON p.channel=c.id WHERE c.id=?1 AND c.retired=0 AND p.workspace=?2 AND (p.identity=?3 OR p.identity='*') LIMIT 1", params![channel, actor.route.workspace, actor.route.identity], |r| r.get(0)).optional()?;
    let kind = kind.context("channel unavailable")?;
    if membership || kind == "fixed" {
        member(db, actor, channel)?;
    }
    Ok(kind)
}
impl Store {
    // Commit bounded maintenance separately: a subsequent quota refusal must
    // not roll back the expiry progress required to admit the next request.
    fn cleanup(&mut self, now_ms: u64) -> Result<()> {
        let tx = self
            .db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        tx.execute("DELETE FROM receipts WHERE rowid IN (SELECT rowid FROM receipts WHERE expires<=?1 ORDER BY expires LIMIT 256)", [number(now_ms)?])?;
        tx.execute("DELETE FROM sends WHERE rowid IN (SELECT rowid FROM sends WHERE expires<=?1 ORDER BY expires LIMIT 256)", [number(now_ms)?])?;
        tx.commit()?;
        Ok(())
    }
}

#[cfg(test)]
#[path = "durable/tests.rs"]
mod tests;
