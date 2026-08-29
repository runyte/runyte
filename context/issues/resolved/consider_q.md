---
title: "Pane, buffer, and editor exit commands have inconsistent scopes and safety"
status: resolved
reported: 2026-08-12
resolved: 2026-08-12
legacy_commit: aaeab75
---

## Resolution

Commit `aaeab75` (`Clarify pane and buffer closing commands`) resolves the
issue. `execute_colon_invocation` previously sent `:q` directly to the global
`request_quit` path, and `:wq` unconditionally called that same path after a
save. `close_active_buffer` had a separate confirmation flow that eventually
called `close_buffer_and_pane`, coupling buffer retirement to layout changes.
The prompt control handler also retained an undocumented `Ctrl-q` route to the
global quit path even though the normal-mode quit binding had been removed.

The commands now have distinct scopes. `:c` closes the active pane and still
refuses the last pane. `:q` closes the active pane when there are several and
leaves the editor, or detaches an attached TUI, only from the last pane;
`:qa` is the explicit all-pane exit. Their `!` forms bypass the relevant
last-pane or all-pane safety check. `Space w c` remains the registry-backed
binding for `:c`, and no key binding exits the editor.

`:buffer-close` / `:bc` retires the active buffer without changing the pane
layout. Every pane showing that buffer selects the next live buffer in arena
order, wrapping at the end; when no other buffer exists, one scratch buffer is
created as the replacement. The safe command refuses unsaved text instead of
opening a discard confirmation. `:buffer-close!` / `:bc!` is the explicit
discard path and intentionally has no key binding. `Space b c` invokes the safe
form. The old `:close-buffer` and `:cb` spellings remain compatibility aliases
for `:bc`.

The internal Git commit-message buffer keeps its deliberate write semantics:
`:w` commits the index, closes the message, and returns the pane to its origin.
`:wq` does the same thing in this special buffer and does not continue by
quitting the newly revealed view. Closing an unchanged message with `:bc`, or
discarding an edited message with `:bc!` or `:q!`, cancels the commit and leaves
the index untouched; safe `:bc` and `:q` refuse to discard an edited message.
For ordinary files, `:wq` writes and applies `:q`, while the new `:wbc` writes
and closes the buffer in place. This gives `$EDITOR --wait` callers a way to
complete a requested file without detaching an existing TUI. The undocumented
prompt-mode `Ctrl-q` path was removed.

A 2026-08-28 persistent-session lifecycle follow-up made the existing
editor-exit meaning literal in both deployment modes. `:q` from the last pane
and `:qa` from any layout now stop a persistent session after its shutdown
guards pass. `:detach` is the separate operation that leaves the host and all
editor state running.

A same-day delivery follow-up kept the quitting interactive connection in the
host's active set after queuing `ShuttingDown` (or the directory-bearing
detach-shaped response used by `:quit-here`). The final connection flush can
now observe both its sender and identity and keeps the runtime alive until the
terminal response is written, instead of allowing a fast shutdown to truncate
an otherwise successful quit on macOS. Coverage is provided by
`an_interactive_quit_flushes_its_shutdown_response_without_a_control_client`
in `tests/local_protocol.rs`.

The same follow-up made
`quit_here_reports_its_directory_to_a_handoff_capable_client` in
`tests/persistent_host.rs` obtain an idle complete frame before invoking the
command. Protocol frames are optimistic-concurrency tokens; using the first
startup frame while Git discovery was still active let the discovery result
advance the host and reject `:quit-here` as stale on a loaded runner.

A later same-day follow-up fixed the last ordering race inside the connection
task. Its fair read/write selection could accept a periodic wait-status poll
ahead of lifecycle replies that were already queued while the host was
shutting down. Because the request queue was no longer being serviced, the
client could see the socket close before receiving `WaitState` and
`ShuttingDown`. The bounded semantic response queue now has priority over
socket reads, which makes the existing shutdown flush deterministic. The
direct interactive regression now also waits for an idle frame before
invoking `:quit`, so startup Git discovery cannot make its concurrency token
stale.

A 2026-08-29 delivery follow-up separated final lifecycle responses from the
bounded semantic queue. `detach_client` previously ignored a failed
`try_send(Detached)` and immediately dropped the connection's last response
sender. If ordinary semantic replies filled that queue, a valid explicit
detach therefore arrived at the client as clean EOF. `Detached`,
`SwitchWorkspace`, and `ShuttingDown` now use a dedicated one-response lane;
the connection drains earlier semantic replies first, then the final response,
before observing that all senders are closed. The transport regression
`final_response_survives_a_full_semantic_queue` fills the ordinary queue and
proves `Detached` still precedes EOF.

This deliberately follows the Vim/Helix distinction between a view-local
`:q`, an all-view `:qa`, and a buffer-local `:bc`, while retaining Runyte's
safer `:c` that cannot close the last pane. It also retains Runyte's special
`:w`-means-commit behavior for its internal commit buffer rather than requiring
`:wq` there.

Coverage is provided by
`app::tests::quit_closes_one_pane_while_quit_all_leaves_the_editor`,
`app::tests::control_q_does_nothing_inside_the_command_prompt`,
`app::tests::closing_a_buffer_keeps_every_pane_and_selects_the_next_buffer`,
`app::tests::closing_a_shared_buffer_retargets_every_view_without_closing_one`,
`app::tests::closing_a_modified_buffer_requires_the_force_command`,
`app::tests::closing_a_commit_message_abandons_it_and_stages_nothing_differently`,
`app::tests::write_quit_commits_without_quitting_from_a_commit_message`, and
`app::tests::quitting_an_edited_commit_message_requires_force_and_never_commits`
in `src/app.rs`; `close_bindings_keep_panes_and_buffers_as_separate_decisions`
in `tests/keymap.rs`; and
`git_commit_wait_closes_its_buffer_without_detaching_an_existing_tui` plus
`git_commit_wait_tui_completes_through_write_quit` in
`tests/local_protocol.rs`.

## Report

Runyte used `:q` to close the editor completely, `:c` to close the current
pane, and `:cb` to close the current buffer. `:cb` also closed the current pane
when more than one pane existed. `Space w c` invoked `:c`, `Space b c` invoked
`:cb`, and `:q` had to be typed manually so the editor could not be exited by
mistake.

`:q!` could exit even when buffers were not saved. `:c` did not need a force
variant because it could not close the last pane and closing a pane did not
close its buffer. `:cb`, however, could close a buffer and discard unsaved work,
which made its behavior dangerous.

The desired command model was:

- `Space w c` maps to `:c`.
- `Space b c` maps to `:bc`.
- `:bc!` is available only as a typed command.
- `:bc` does not close a pane. It displays the next buffer in the same pane,
  or a scratch buffer when there are no other buffers.
- `Ctrl-q` is removed.

In Runyte's internal Git commit-message buffer, `:w` wrote the message, closed
the buffer, and committed the staged changes. This differed from Neovim, where
`:wq` would normally be used. The V8 `$EDITOR --wait` workflow also introduced
`:wq` as the documented completion command:

> $EDITOR --wait workflow
> Runyte can be used by programs that need an editor to block until editing is
> finished—for example Git commit messages. Saving and using :wq completes the
> request correctly without shutting down an existing workspace.

The internal commit buffer should keep `:w` as its commit action. Leaving that
buffer without saving should cancel the commit. Whether `:wq` in that special
buffer should also exit Runyte was initially undecided.
