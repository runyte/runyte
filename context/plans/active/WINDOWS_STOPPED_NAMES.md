# Native stopped rename: one authoritative stored name

Reviewed design for package 4e.3f, implemented in `8d5c009` and natively
validated at the [continuation checkpoint](../../reviews/windows_phase2_handoff.md).
This record defines one authoritative name store; history remains a cache.

## Authority

While a host is live, its exact publication metadata owns its observable name;
the host's existing Publication::rename owns the live update. Never overlay a
hidden live publication with a locally inferred NameStore or history name.

When a configured project is stopped, NameStore owns its explicit name and the
name prepare_named will use on the next startup. recent_history.name is only a
cache/default fallback. Therefore stopped rename needs one owned-file ledger,
not a cross-file authoritative transaction. Verified NameStore commit is success,
even if a later history-cache refresh fails. Unix behavior remains unchanged.

## Narrow operation and locks

An owned native catalog worker performs a synchronous stopped-name transaction
using an explicit configured EndpointLocation, NameStore and captured history
selection. It keeps recovery state across request cancellation or operation
failure. At most one pending name ledger is retained; another rename refuses
until recovery completes or unresolved state is explicitly handled.

Acquire configured workspace identity locks, deterministic registry locks, then
the selected stable NameStore lock. If checking current history membership or
fallback collisions under its lock, acquire that history lock last. History-only
transactions acquire no endpoint or name locks. Never await a pipe while holding
these guards, and never invert the order by starting from a history lock.

Before mutation, reread the exact configured ready record and configured namespace
and own inventory keys. Missing ready alone is insufficient. The minimal API
refuses every occupied, malformed, denied or otherwise indeterminate record.
Separate explicit stale recovery may run beforehand; then reacquire and repeat
vacancy proof. prepare_named uses the same lock prefix, preventing startup between
vacancy proof and commit.

The selected exact native project identity must still correspond to an existing
history entry. Do not canonicalize a missing selected project into a replacement,
insert a concurrently forgotten history row, or derive roots from hidden metadata.
Recheck any captured expected stored name before mutating; a newer explicit rename
is a stale intent. Preserve unrelated history visits, ordering and number changes.

## One-record replacement and recovery

Reuse windows_endpoint/names.rs Change mechanics for the stored-name leaf: retain
old admitted file identity/bytes, register pending ownership before writing, stage
and sync, remove only that old identity, install without replacement, and verify
the advertised pathname/file identity/bytes before success. Preserve a racing
foreign replacement. The existing 1 KiB name bound remains sufficient; no history
8 MiB payload or second authoritative record enters the ledger.

On failure attempt owned rollback. Retain old/new and at most one restoration
generation for retry; reuse an already installed restoration identity. Report an
unresolved single-file failure as unknown outcome, not a committed rename. Caller
cancellation does not abandon begun mutation or its ledger. Later recovery must
reacquire guards and recheck vacancy: a newly started host now owns live naming.
Do not claim crash atomicity, a multi-file journal or power-loss durability.

## Read-only effective names

For configured remembered stopped projects, read NameStore before row naming or
default allocation. Precedence is live publication name for a live entry;
configured stored name for a stopped entry; then cached history/default only if
the stored name is absent. A stale cache is not an authority conflict.

Current NameStore::open/load cannot serve read-only catalog discovery: open creates
and hardens storage, and load appends a stable lock file. Add a narrow open-existing
reader that creates neither directories nor records nor locks, pins admitted
identities and coordinates through an existing stable lock. Missing storage means
fallback; malformed, denied, replaced or busy storage means uncertainty. A present
name with an unverifiable/missing lock refuses. An entirely absent name/store can
be observed absent without creating it; a later commit is a later observation.
Do not interpret the unlink/install gap as absence while its existing lock is held.

History refresh may cache the verified effective name using a fresh locked history
value and existing field compare guards. It must preserve unrelated updates and
not recreate a forgotten row or overwrite a newer cached value. Cache failure can
be reported separately but cannot undo/misreport verified rename success. Even if
an old refresh leaves a stale cache, the next effective-name read still uses the
stored name. No whole-history rollback is needed.

## Name uniqueness

Existing namespace collision checks remain authoritative under registry locks.
Shared secondary namespaces participate; owner-wide inventory does not reserve
names across isolated namespaces. Conflicting or incomplete observations refuse.
Two explicitly included isolated hosts may still have the same name, and public
selectors must report ambiguity.

Local default allocation overlays valid stored names before choosing generated
names. Cached names may conservatively reserve a candidate and cause refusal;
they must never overwrite a configured authoritative name. Existing duplicate
explicit names remain ambiguity/conflict, not permission to silently rename one.
Native explicit stopped rename must route through NameStore; a history-only rename
helper may maintain the cache but must not be exposed as the actual rename action.

Inspect other configured stores one at a time before holding the selected store
lock, avoiding arbitrary nested per-project lock order. Shared cache namespaces
already share registry serialization for native explicit name mutations. Recheck
latest fallback-name competitors under the last-acquired history lock before
commit. If a consistent bounded availability observation cannot be made, refuse.
A future requirement for simultaneous multi-store locking needs deterministic
native identity ordering; it does not justify a cross-file name/history journal.

Forgetting history leaves the explicit NameStore name in place, so revisiting the
project recovers the name startup already uses. Missing stored names continue to
use history/default fallback. A malformed stored name never silently falls back.

## Acceptance

- Deliberately fail cache refresh after successful rename: list/select and next
  prepare_named still use the new authoritative name.
- Read-only refresh creates no directory or lock; absent state falls back while
  malformed, busy and replaced state conservatively fails.
- Stored overrides participate in default allocation/collision checks; stale
  cache never overwrites them; isolated publications remain separately selectable.
- Startup racing rename is serialized; ready-only and namespace occupants,
  incomplete scans and forgotten/stale selected history refuse safely.
- Partial staging/install/restore, foreign replacement and cancelled observers
  preserve bounded ownership/recovery with no falsely acknowledged commit.
- Concurrent history visits/numbers/forget/cache updates survive; no obsolete
  whole-history snapshot is restored.
- Live metadata and hidden inventory provenance remain authoritative; configured
  stored-name decoration is confined to proven stopped projects.
