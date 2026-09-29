# Terminal numbers grow without bound

A terminal pane's title reads `[terminal #<id>] <name>`, where `<id>` is the
terminal's `TerminalId` (`src/snapshot.rs`, `terminal_title`). The same value
is what `:terminal-send` and `:terminal-show` accept through
`TerminalSessions::resolve`, and what the Navigator, the terminal list and the
Finder match as `3` or `#3` (`src/app/navigation_workflows.rs`).
`TerminalSessions` assigns ids from a counter that only increases, so after a
day of opening and closing shells a new terminal is `#47` even when it is the
only one running. Exited terminals keep their number and show it in the
terminal list preview.

Expected: the number shown to the user stays small. A running terminal holds a
display number, and a new terminal takes the lowest number not held by another
running terminal. A terminal gives its number up when its child exits or when
it is closed; an exited terminal is unnumbered. This matches persistent
sessions, where only running sessions are numbered and a stopped row shows no
digit.

## Constraints

- `TerminalId` must not be reused. Its documentation relies on that: a pane
  or record holding the id of a closed terminal learns that the terminal is
  gone rather than finding a different one. Ids are held past a terminal's
  lifetime by pane content and `OpenDestination::Terminal` history,
  pending terminal handoffs (`src/terminal/pending.rs`), plugin invocations
  (`src/plugin/application.rs`), and terminal-text proposals awaiting
  approval (`src/workspace/host/context.rs`). A recycled id would let an
  approval given for one terminal deliver text to another. The display number
  is therefore a separate value beside the id, not a replacement for it.
- Plugins, the external context profile, and the bridge keep addressing
  terminals by the stable id or by the opaque handles derived from it. The
  display number is a user-facing label only.
- The persistent-session host owns number assignment, so a terminal keeps its
  number across detach and reattachment and every attached client shows the
  same number.
- A number is released on exit and on close, and reassigned only when a new
  terminal starts.

## Behavior to define

- Pane titles read `[terminal #<number>] <name>` for running terminals. An
  exited terminal no longer appears in a pane title, since its pane reveals a
  buffer on exit; its list row and preview show no number.
- `:terminal-send <n>` and `:terminal-show <n>` resolve `<n>` against running
  terminals' display numbers. A number held by no running terminal is
  reported as not existing. Exact-name resolution is unchanged.
- Navigator, terminal-list and Finder matching of `3` and `#3` use the display
  number. Exited terminals are found by name, title or launch command.
- An exited terminal is no longer addressable by number from the command line.
  Several exited terminals often share a name such as a shell's prompt title,
  and `resolve` rejects duplicate names, so such a terminal is reachable only
  from the terminal list. This is accepted: exited terminals are kept for
  review, not for further input.
- A number read from a title can refer to a different terminal once the
  original exits and a new one starts. The user guide says that numbers
  identify running terminals for interactive use and are not stable across a
  terminal's exit.

## Reproduction

1. Open a terminal (`[terminal #1]`), exit its shell, and close it from the
   terminal list.
2. Repeat several times.
3. Open another terminal while no other terminal is running. Its title shows
   the next counter value, such as `[terminal #5]`, rather than `#1`.

## Documentation

Update the terminal sections of `docs/user-guide.md` (the Navigator
description of the title ID, and the `[terminal #<id>]` paragraph under
Terminals) and the terminal passages of `context/reference/ui-vocabulary.md`.
