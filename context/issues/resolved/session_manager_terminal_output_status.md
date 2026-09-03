---
title: "The session manager does not identify sessions with quiet terminal output"
status: resolved
reported: 2026-09-03
resolved: 2026-09-03
commit: 3f9771b
---

## Resolution

Commit `3f9771b` (`Show quiet terminal output in session manager`) resolved the
issue. The session manager's `WorkspaceRow` previously carried lifecycle and
last-visit facts but no terminal-output activity, while `TerminalSession`'s
existing activity timestamp advanced for arbitrary PTY bytes. Neither value
could answer whether every live terminal had stopped completing output lines.

`Emulator::feed` now reports semantic completed presentation lines to its
owning `TerminalSession`. Line feed, index/new-line controls, delayed automatic
wrap, and top-anchored primary-screen scroll commits advance a creation-based
timestamp. Partial rows, carriage-return rewrites, cursor movement, private
scroll regions, alternate-screen repainting, and resize do not. The host
reduces the live-terminal timestamps to their latest value, which is sufficient
to prove that all live terminals have crossed the threshold while keeping the
health response bounded. Exited terminals are deliberately excluded, and a
new terminal starts a clock without treating its initial empty row as output.

Protocol version 47 carries that single host-owned Unix-second scalar in
`HostResponse::Health`. While the manager is open, its existing maintenance
cadence requests a silent catalog refresh at most once every five seconds; no
poll runs while it is closed, and the manager neither reads terminal contents
nor derives activity from previews. The product interval is five minutes.
Compatible running sessions with at least one live terminal display `QUIET`
only after the latest live-terminal baseline reaches that age. Stopped,
incompatible, terminal-free, and exited-only sessions make no quietness claim.

The picker now renders `Status` immediately after `Last active` as the sixth
column. Both values occupy one pinned trailing run, preserving both semantic
columns when narrow layouts clip the longer identity fields. Poll refreshes
also preserve the selected workspace across catalog reordering.

Regression coverage lives in:

- `src/terminal/emulator.rs`: `completed_line_activity_excludes_partial_rows_rewrites_and_resize`,
  `completed_line_activity_counts_wrap_index_and_scroll_commits`, and
  `completed_line_activity_excludes_private_scrolls_and_alternate_screen_repaints`;
- `src/terminal/mod.rs`: `completed_line_activity_aggregates_only_live_terminals`
  and `only_completed_lines_advance_the_session_activity_baseline`;
- `src/app/tests/workspace.rs`:
  `session_terminal_output_status_requires_every_live_terminal_to_be_quiet`
  and `an_open_session_manager_transitions_into_and_out_of_quiet`;
- `src/ui.rs`:
  `session_manager_draws_column_headers_in_standalone_and_attached_frames` and
  `session_activity_survives_preview_width_clipping`;
- `tests/persistent_host.rs`:
  `terminal_pid_output_and_input_survive_detach_disconnect_and_reattach`.

Known limitation: health timestamps have whole-second precision, and a new
completed line can take up to the five-second manager polling interval to clear
an already visible `QUIET` value.

## Report

The session manager listed the session number, name, branch, path, and
last-active age, but did not expose whether terminal sessions in a workspace
were still producing new lines. A manager used to supervise coding agents
therefore gave no at-a-glance indication that every agent terminal in a
persistent session might have stopped producing output.

The expected presentation was a `Status` column immediately after
`Last active`, with one of two values in each row:

- an empty string while the session does not meet the terminal-output
  inactivity condition;
- `QUIET` when every relevant terminal session has produced no new complete
  output line for the inactivity interval.

`QUIET` was preferred to `STALE`. `[STALE]` already has the precise Runyte
meaning that an ordinary file's path no longer agrees with its accepted disk
baseline. Terminal output that has merely stopped advancing is still valid and
current, so calling it stale would conflate unrelated states. `QUIET` states
the observed condition without claiming that a child process is idle, blocked,
finished, or unhealthy.

The value was required to be display state in the shared session manager, not
a persistent setting owned independently by each host. The report left the
following product decisions open:

- the inactivity interval, which needed to be long enough that ordinary pauses
  would not make the column flicker;
- whether a terminal's initial line, an unterminated partial line, a
  carriage-return rewrite such as a spinner, and terminal resize reflow counted
  as new-line activity, because the requested signal concerned new lines rather
  than arbitrary PTY bytes or screen-cell changes;
- whether only live terminals or both live and exited terminals were relevant,
  with no-terminal sessions remaining empty unless vacuous quietness proved
  useful;
- how often running hosts published or answered the activity value without
  polling every terminal or blocking input and rendering.

A stopped persistent session cannot report live terminal progress and was not
to be labelled `QUIET` merely because no host was running. `Status` also had to
follow `Last active` as a sixth labelled column. Narrow layouts needed to keep
the status readable without losing the existing guarantee that `Last active`
remained visible when identity columns were clipped.

The status had to describe the persistent session as a whole: one relevant
terminal producing new lines kept it empty. It also had to use bounded semantic
state from the owning host, without fetching terminal contents into the manager
or comparing rendered previews. As observational state, it could not alter
terminal-session lifecycle, mark output as read, or affect persistent-host idle
retirement.

Required regression cases were a running session with one active terminal,
multiple terminals where only one continued to append lines, all terminals
past the inactivity interval, no terminals, and a stopped session. Transitions
into and out of `QUIET` while the manager remained open also needed coverage,
including an attached client reading the same host-owned result.
