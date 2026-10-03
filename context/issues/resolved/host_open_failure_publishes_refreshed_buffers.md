---
title: "Host open errors can publish partial buffer changes"
status: resolved
reported: 2026-10-03
resolved: 2026-10-03
commit: 214f74a
---

## Resolution

Commit `214f74a` (`fix(workspace): activate prepared host opens without fallible reopening`).

Host opens previously called the generic path opener after publishing staged and refreshed buffers. A changed path could then reject activation, or reinterpret a prepared directory as a file. The request now rejects known activation refusal before preparation, activates captured file buffer identities through the shared infallible presentation helper, and preserves directory intent through the existing read-before-mutation directory operation. Native opening retains the same activation side effects.

Tests: `host_open_activates_prepared_buffers_without_reopening_changed_paths` and `directory_host_open_refuses_replacement_before_publishing_any_buffer` in `src/app/tests/host_open_atomicity.rs` deterministically replace paths at the preparation boundary. The existing navigation/file tests and protocol atomicity regression also pass.

## Report

An activated host open can fail after changing live buffers. In
`App::host_open_files_with_refresh`, refreshed buffers and newly staged buffers
are published before `open_file` activates the first requested path. That call
can still fail a new pathname identity lookup, despite the request's
all-or-none contract.

For a deterministic reproduction, prepare an activated host open for
`folder/note.txt` and `second.txt`, then replace `note.txt` with a symbolic link
to itself between preparation and publication. The final `open_file` lookup
fails with `too many symbolic links` after both staged buffers have already
been published.
The failed caller receives no successful open result or buffer identities,
although editor state has changed. A wait request can similarly publish clean
buffer refreshes before a changed pathname rejects activation.

Directory activation has the same publication problem when it re-enters the
generic path opener. Prepare a directory-first request for `folder` and
`second.txt`, then rename the directory and put a text file at `folder` before
activation. The opener publishes the file before the subsequent directory
assertion rejects the request. This occurs both from a scratch buffer and from
an existing explorer.

Check activation refusal before preparing or publishing replacements. After
validation, activate the captured buffer identity without another fallible path
lookup, and retain the captured directory intent when activating an explorer.
Preserve ordinary path-opening selection, jump, explorer-view, terminal
return, launch-selection, and language-service behavior. Failed requests must
preserve buffer contents, revisions, mode, and selection.
