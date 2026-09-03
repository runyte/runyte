# The session manager does not identify sessions with quiet terminal output

The session manager lists the session number, name, branch, path, and
last-active age, but it does not expose whether terminal sessions in a
workspace are still producing new lines. A manager used to supervise coding
agents therefore gives no at-a-glance indication that every agent terminal in
a persistent session may have stopped producing output.

## Expected behavior

Add a `Status` column immediately after `Last active`. Each row contains one of
two values:

- an empty string while the session does not meet the terminal-output
  inactivity condition;
- `QUIET` when every relevant terminal session has produced no new complete
  output line for the agreed inactivity interval.

`QUIET` is preferable to `STALE`. `[STALE]` already has the precise Runyte
meaning that an ordinary file's path no longer agrees with its accepted disk
baseline. Terminal output that has merely stopped advancing is still valid and
current, so calling it stale would conflate unrelated states. `QUIET` states
the observed condition without claiming that a child process is idle, blocked,
finished, or unhealthy.

The value is display state in the shared session manager, not a persistent
setting owned independently by each host.

## Points the design has to settle

- The inactivity interval after the last newly completed terminal line is not
  yet specified. It must be long enough that ordinary pauses do not make the
  column flicker.
- Decide whether a terminal's initial line, an unterminated partial line, a
  carriage-return rewrite such as a spinner, and terminal resize reflow count
  as new-line activity. The report specifically asks about new lines rather
  than arbitrary PTY bytes or screen-cell changes.
- Decide which terminal sessions are relevant: live terminals only or live and
  exited terminals. Sessions with no terminals should remain empty unless an
  explicit product decision gives vacuous quietness a useful meaning.
- A stopped persistent session cannot report live terminal progress and should
  not be labelled `QUIET` merely because no host is running.
- Determine how often running hosts publish or answer the activity value so the
  manager can update without polling every terminal or blocking input and
  rendering.

## Constraints

- `Status` follows `Last active`, as a sixth labelled column. Narrow layouts
  must preserve a readable terminal-output status without losing the existing
  guarantee that `Last active` remains visible when identity columns are
  clipped.
- The status must describe the session as a whole. It becomes `QUIET` only
  when all relevant terminal sessions meet the chosen condition; one terminal
  producing new lines keeps it empty.
- The implementation must use bounded semantic state from the owning host. It
  must not fetch terminal contents into the manager or infer activity by
  comparing rendered previews.
- The status is observational. It must not alter terminal-session lifecycle,
  mark output as read, or affect persistent-host idle retirement.

## Regression coverage

Cover a running session with one active terminal, multiple terminals where only
one continues to append lines, all terminals past the inactivity interval, no
terminals, and a stopped session. Also cover transitions into and out of
`QUIET` while the manager remains open, including an attached client reading
the same host-owned result.
