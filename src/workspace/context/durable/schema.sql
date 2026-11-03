-- SPDX-License-Identifier: MPL-2.0
CREATE TABLE participants (
 id TEXT PRIMARY KEY, secret_hash TEXT NOT NULL UNIQUE,
 provider TEXT NOT NULL, task TEXT NOT NULL
) STRICT;
CREATE TABLE bindings (
 participant TEXT NOT NULL REFERENCES participants(id),
 workspace TEXT NOT NULL, identity TEXT NOT NULL,
 PRIMARY KEY(participant, workspace, identity)
) STRICT;
CREATE TABLE channels (
 id TEXT PRIMARY KEY, name TEXT NOT NULL, kind TEXT NOT NULL CHECK(kind IN ('public','fixed')),
 revision INTEGER NOT NULL DEFAULT 1, retired INTEGER NOT NULL DEFAULT 0
) STRICT;
CREATE TABLE policy (
 channel TEXT NOT NULL REFERENCES channels(id), workspace TEXT NOT NULL, identity TEXT NOT NULL,
 PRIMARY KEY(channel, workspace, identity)
) STRICT;
CREATE TABLE members (
 channel TEXT NOT NULL REFERENCES channels(id), participant TEXT NOT NULL REFERENCES participants(id),
 PRIMARY KEY(channel, participant)
) STRICT;
CREATE TABLE messages (
 seq INTEGER PRIMARY KEY AUTOINCREMENT, channel TEXT NOT NULL REFERENCES channels(id),
 sender TEXT NOT NULL REFERENCES participants(id), text TEXT NOT NULL,
 location TEXT NOT NULL, at_ms INTEGER NOT NULL
) STRICT;
CREATE INDEX channel_sequence ON messages(channel, seq);
CREATE TABLE pending (
 channel TEXT NOT NULL REFERENCES channels(id),
 participant TEXT NOT NULL REFERENCES participants(id), seq INTEGER NOT NULL REFERENCES messages(seq) ON DELETE CASCADE,
 PRIMARY KEY(participant, channel, seq)
) STRICT;
CREATE INDEX pending_sequence ON pending(seq);
CREATE TABLE receipts (
 id TEXT PRIMARY KEY, participant TEXT NOT NULL REFERENCES participants(id),
 channel TEXT NOT NULL REFERENCES channels(id), digest TEXT NOT NULL,
 expires INTEGER NOT NULL, acknowledged INTEGER NOT NULL DEFAULT 0,
 UNIQUE(participant, channel, digest)
) STRICT;
CREATE INDEX receipt_expiry ON receipts(expires);
CREATE TABLE receipt_items (
 receipt TEXT NOT NULL REFERENCES receipts(id) ON DELETE CASCADE,
 seq INTEGER NOT NULL REFERENCES messages(seq) ON DELETE CASCADE,
 PRIMARY KEY(receipt, seq)
) STRICT;
CREATE INDEX receipt_sequence ON receipt_items(seq);
CREATE TABLE sends (
 participant TEXT NOT NULL REFERENCES participants(id), channel TEXT NOT NULL REFERENCES channels(id),
 key TEXT NOT NULL, digest TEXT NOT NULL, seq INTEGER NOT NULL, expires INTEGER NOT NULL,
 PRIMARY KEY(participant, channel, key)
) STRICT;
CREATE INDEX send_expiry ON sends(expires);
PRAGMA user_version = 1;
