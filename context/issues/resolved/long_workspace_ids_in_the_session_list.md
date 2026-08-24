---
title: "The session listing printed full 32-character workspace IDs, wrapping on narrow terminals"
status: resolved
reported: 2026-08-21
resolved: 2026-08-21
legacy_commit: cc28c36
---

## Resolution

Commit `cc28c36` (`Abbreviate workspace IDs in the session listing`) shortens
what `--session-list` prints, and only that. `list_sessions` in `src/main.rs`
built each row's first cell from `workspace.id.clone()`, the whole value
`workspace::transport::workspace_id` produces — the first 32 characters of the
SHA-256 of the encoded canonical project root. That is already a truncation
chosen arbitrarily, and nothing reads it in full: `resolve_known_workspace_from_rows`
(`src/workspace/catalog.rs`) and `resolve_registered_host_from`
(`src/workspace/lifecycle.rs`) both fall back to `starts_with` for any hex
selector, so an ID gets shortened again by whoever types it.

`catalog::abbreviated_id_width` returns the narrowest prefix width that keeps
the given IDs distinct. It starts at the new `ABBREVIATED_WORKSPACE_ID`
constant, six, and widens one character at a time only while two of the listed
IDs still read the same, stopping at the longest ID present. Six hex digits
separate far more workspaces than one person keeps open, and the width is
computed per listing rather than fixed so that a row can never print an ID
that resolves to a different row. `list_sessions` computes the width once
across the rows it is about to print and slices each ID to it.

The stored identity was deliberately left alone. Lowering `HOST_ID_LENGTH`
instead would have been a far larger change than the problem warranted:
`transport.rs` validates `metadata.id.len() == HOST_ID_LENGTH` when reading a
registration, and the ID names the endpoint and registry files, so every
already-registered host would have become unrecognizable and any running one
orphaned while still holding the endpoint its clients resolve. Abbreviating
only the display leaves `HOST_ID_LENGTH`, the registry, and the endpoint paths
exactly as they were.

This follows what Runyte already does for Git object IDs, where a row shows an
abbreviated hash that is itself a usable selector.

The listing narrowed from 125 to 99 columns for the reported four workspaces.

Tests: `abbreviated_ids_stay_six_characters_while_they_tell_workspaces_apart`,
`abbreviated_ids_grow_only_far_enough_to_separate_a_shared_prefix`,
`ids_that_never_separate_abbreviate_to_their_whole_length`, and
`an_abbreviated_id_still_resolves_to_the_row_it_was_printed_from` in
`src/workspace/catalog.rs`, the last of which feeds a printed abbreviation back
through `resolve_known_workspace_from_rows` and checks it reaches the row it
came from. `sessions_list_rename_restart_and_resolve_by_id_name_or_directory`
in `tests/persistent_host.rs` now asserts that a real listing carries the
six-character ID and not the full one, while still renaming through a
twelve-character prefix selector.

Known limitation: an abbreviation is unique across the listing it was printed
from, not across every workspace that will ever exist. A later selector
resolves against whatever is registered at that moment, so a workspace first
recorded after a listing was read could share an abbreviation with one of its
rows. That needs two project roots whose hashes agree over six hex digits, and
`resolve_known_workspace_from_rows` and `resolve_registered_host_from` both
answer several prefix matches by calling the selector ambiguous, so such a
collision costs an error rather than acting on the wrong workspace. This is the
same trade-off an abbreviated Git object ID carries.

Only the ID column was narrowed. `DIRECTORY` is still
unbounded and prints absolute paths without abbreviating `$HOME`, and the
`TERMINALS` and `WAITING` headers are wider than any count they hold, so a
listing of deeply nested projects can still wrap. Rows for stopped workspaces
also still end in the trailing spaces that padding the final cell produces,
which predates this change.

## Report

`runyte --session-list` (`ru -l`) showed full-length workspace IDs:

```
ID                                NAME             DIRECTORY                        STATE    UNSAVED  TERMINALS  WAITING  TUI
--------------------------------  ---------------  -------------------------------  -------  -------  ---------  -------  ---
96ceecd6a1f66da1b4ef385dbb62328a  runyte           /home/user/code/runyte           running  0        0          0        no
7862cb247950d6d2435bd7545273d79f  runyte-terminal  /home/user/code/runyte-terminal  stopped
22b80e1b3b4ca1b84282af9e467983de  user             /home/user                       stopped
d4db0b5604ea856609369870185fc36a  runyte-dev       /home/user/code/runyte-dev       stopped
```

On a narrow terminal, such as one half of a split, the output wrapped. More
than 100 open workspaces is not an expected number, so IDs that long were not
needed to tell them apart. The agreed behavior was to abbreviate the ID to six
characters in the listing and leave the stored workspace identity at its
current length.
