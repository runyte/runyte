---
title: "Buffers, panes, and terminals had overlapping close and navigation lifecycles"
status: resolved
reported: 2026-08-20
resolved: 2026-08-20
legacy_commit: 815c7a3
---

## Resolution

Commit `815c7a3` (`Separate buffer pane and terminal lifecycles`) separated the
three resource lifecycles. `App::rebuild_buffer_picker` and the project-finder
resource projection had enumerated every live arena slot, so detached generated
views accumulated as indistinguishable buffer noise. It introduced explicit
discoverability rules: clean special buffers and empty clean scratch buffers
retired after their final pane left, dirty special buffers remained reachable,
and explorer rows identified both their role and directory.

The later `Retain two recent special buffers` follow-up adjusted the clean
special-buffer rule without returning to unbounded accumulation. The two most
recently active clean special buffers now stay reachable for back/forward
navigation, and the least recent detached one retires when a third is
activated. Empty clean scratch and dirty special-buffer behavior is unchanged.

`App::close_buffer` had selected one fallback by arena order for every pane,
while `App::request_view_quit` treated quitting as a pane-only operation. Buffer
retirement now uses a per-pane most-recent-buffer history, preserves terminal
jumps with a replacement backing buffer, and keeps every pane in place.
`:close[!]` / `:c[!]` own that operation. `:quit[!]` / `:q[!]` retire a buffer
only when the closing pane was its final visible owner, and leave shared
buffers open.

Terminal termination was reachable through the ordinary close/quit surfaces
and through direct key and colon commands. Those direct commands were removed;
the terminal manager's Close action and child exit are now the explicit
termination paths. A child exit preserves its pane, pane quit leaves the child
headless, every standalone editor-exit spelling refuses live terminals even
with `!`, and persistent quit continues to detach without signalling children.

Terminal Insert dispatch had treated both `Ctrl-\\` and `Ctrl-w` as Normal-mode
exits. `Ctrl-\\` (including legacy `Ctrl-4`) is now the sole exit. `Ctrl-w`
starts a registry-backed movement-only prefix in Insert mode; a terminal
destination remains Insert, a document destination becomes Normal, and
canceling the prefix keeps terminal input active. Deliberately, literal
`Ctrl-w` is no longer sent through a special escape command because the chord
is reserved for pane movement.

Coverage lives in `src/app.rs` tests
`an_empty_clean_scratch_buffer_retires_after_its_last_view_leaves`,
`the_two_most_recent_clean_special_buffers_remain_jumpable`,
`opening_a_third_clean_special_buffer_retires_the_least_recent_detached_one`,
`an_async_special_view_precedes_the_buffer_reached_by_immediate_history_navigation`,
`a_dirty_special_buffer_remains_discoverable_after_its_last_view_leaves`,
`closing_a_shared_buffer_uses_each_panes_own_recent_history`, and
`quit_closes_an_exclusive_buffer_but_keeps_a_shared_one`; in `src/jumplist.rs`
test `retiring_a_buffer_preserves_terminal_surfaces_with_a_live_backing`; in
`tests/keymap.rs` test
`control_backslash_exits_insert_and_control_w_moves_between_panes`; in
`tests/key_hints.rs` test
`terminal_control_w_starts_the_insert_pane_navigation_prefix`; and in
`tests/terminal.rs` tests
`close_refuses_a_terminal_and_quit_only_removes_its_pane`,
`quitting_refuses_a_running_terminal_even_when_forced`, and
`exiting_a_terminal_preserves_its_pane_when_another_pane_exists`.

## Report

Buffer, pane, and terminal closing needed three separate lifecycles.

`Space b b` and the resource mode reached with `Space f Tab` needed to display
an explorer as `[explorer] dirname` followed by its project-relative path,
typically `.` for the project root.

Clean special buffers needed to close automatically once no pane displayed
them. Dirty special buffers needed to remain open and discoverable rather than
being discarded implicitly. An empty, clean scratch buffer likewise needed to
disappear when its final pane switched to another buffer or terminal.

`:close[!]` (`:c[!]`) needed to close the active buffer without closing any
pane. The safe spelling had to refuse unsaved changes; the force spelling could
discard them. Every pane that displayed the closed buffer needed to show its
own most recent live buffer, or a scratch buffer when its history had no live
entry. Closing a terminal with either spelling had to be refused.

`:quit[!]` (`:q[!]`) needed to close the active pane. If that pane was the last
one displaying its buffer, the buffer needed to close too; a buffer still
displayed in another pane needed to remain open. Unsaved text being retired
required the force spelling. A terminal displayed in the pane needed to remain
running without a pane. From the last pane, quit retained its application-exit
or persistent-detach meaning.

Terminals could end only when their child exited, including a shell receiving
`exit`, or through the explicit Close action in `:terminals` (`Space t t`). The
direct `Space t k` and `:terminal-close` surfaces needed removal. Ending a
displayed terminal needed to preserve its pane and reveal that pane's most
recent buffer, or a scratch buffer. `:quit[!]`, `:quit-all[!]`, and
`:quit-here[!]` could never signal a terminal. Persistent clients could detach
while terminals continued in the host; standalone exit needed to refuse while
a terminal was live, including for a force spelling.

Terminal Insert mode needed to enter Terminal Normal mode only with `Ctrl-\\`
(and its legacy `Ctrl-4` representation). `Ctrl-w` instead needed to begin pane
navigation immediately. Moving to another terminal preserved Insert mode;
moving to a document landed in Normal mode so the next key could not edit it
accidentally. Canceling the prefix left terminal input active.
