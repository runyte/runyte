---
title: "Terminal numbers grew without bound"
status: resolved
reported: 2026-09-29
resolved: 2026-09-29
commit: d0a98f8
---

## Resolution

Commit `d0a98f8` (`Number running terminals with recyclable display numbers`)
separated the number a person reads from the terminal's identity. Before it,
`terminal_title` in `src/snapshot.rs`, the destination rows in
`src/app/navigation_workflows.rs`, `terminal_finder_item` in
`src/app/picker_workflows.rs`, and `TerminalSessions::resolve` all used
`TerminalId`, which `TerminalSessions` hands out from a counter that only
increases. The id is deliberately never reused, so the number shown grew for
as long as the workspace lived.

`TerminalSession` now carries `number: Option<u32>`. `TerminalSessions::open`,
`install_prepared` in `src/terminal/pending.rs`, and the test helper
`insert_test_session` assign it through `free_number`, the lowest number no
running terminal holds. A prepared handoff is numbered when it is installed,
not when it is prepared, so a cancelled handoff never holds a number.
`TerminalSessions::apply` clears the number when the child exits, and `close`
releases it by removing the session. Because numbers are assigned by the
`TerminalSessions` the persistent-session host owns, every attached client
sees the same number and a number survives detach and reattachment.

`resolve` matches a numeric target against running terminals' numbers only and
reports `no running terminal is numbered N` otherwise. Exact-name resolution
and the refusal of duplicate names are unchanged. The pane title reads
`[terminal #<number>] <name>` while the child runs and `[terminal] <name>` once
it has exited. The Navigator and terminal-list previews, the Finder item's
detail, the force-kill confirmation, the agent terminal-insert approval, and
the rename status show the number when there is one and nothing otherwise; the
list rows themselves do not, and the list and Finder filters answer to both
`N` and `#N`.
The fallback names used when a closed terminal's session is already gone no
longer print the raw id. Command usage reads `<number|name>`, and the
`ShowTerminal` description says "number or name".

`TerminalId` stays the identity wherever a reference outlives what is on
screen: pane content, `OpenDestination::Terminal`, the jumplist, pending
handoffs, plugin invocations, terminal-text proposals awaiting approval, and
the `terminal-output:{id}` generated view. Recycling the id itself would have
let an approval given for one terminal deliver text to a later one.

The user guide, `context/reference/ui-vocabulary.md`, and
`context/reference/helix-keymap-v1.md` describe the numbers, their release on
exit and close, and that a number identifies what is running now rather than a
terminal for later use.

Tests:

- `terminal_numbers_are_recycled_and_released_on_exit` and
  `explicit_terminal_targets_prefer_numbers_and_refuse_ambiguous_names` in
  `src/terminal/mod.rs`.
- `pending_terminal_fast_output_is_gated_until_install_and_arguments_remain_literal`
  in `src/terminal/pending/tests.rs` covers numbering on installation and
  release on exit.
- `the_terminal_list_finds_running_terminals_by_number_and_exited_ones_by_name`
  in `src/app/tests/session_navigation.rs` covers list filtering, the exited
  title, and a new terminal taking `#1` while its id is the third handed out.
- `sending_buffer_text_chooses_one_terminal_and_names_why_it_cannot` and
  `the_terminal_list_describes_each_session_and_says_so_when_there_are_none`
  in `src/app/tests/commands.rs`.
- `project_finder_matches_the_number_a_terminal_shows` in
  `src/app/tests/search_and_pickers.rs` covers the Finder's `#N` detail and
  filter, and an exited terminal's unnumbered detail.
- `the_pane_is_named_by_the_title_the_child_sets` and
  `force_kill_requires_confirmation_for_visible_and_hidden_stubborn_terminals`
  in `tests/terminal.rs` start their terminal after another has been closed,
  so the expected `#1` differs from the terminal's id.

Known limitation: an exited terminal is no longer addressable by number from
the command line. Several exited terminals often share a name, such as a
shell's prompt title, and `resolve` refuses duplicate names, so such a terminal
is reachable only from the terminal list or the Finder. A number read from a
title can also refer to a different terminal once the original exits and a new
one starts; the user guide says not to record numbers in macros or scripts.

## Report

A terminal pane's title read `[terminal #<id>] <name>`, where `<id>` was the
terminal's `TerminalId` (`src/snapshot.rs`, `terminal_title`). The same value
was what `:terminal-send` and `:terminal-show` accepted through
`TerminalSessions::resolve`, and what the Navigator, the terminal list and the
Finder matched as `3` or `#3` (`src/app/navigation_workflows.rs`).
`TerminalSessions` assigned ids from a counter that only increased, so after a
day of opening and closing shells a new terminal was `#47` even when it was the
only one running. Exited terminals kept their number and showed it in the
terminal list preview.

Expected: the number shown to the user stays small. A running terminal holds a
display number, and a new terminal takes the lowest number not held by another
running terminal. A terminal gives its number up when its child exits or when
it is closed; an exited terminal is unnumbered. This matches persistent
sessions, where only running sessions are numbered and a stopped row shows no
digit.

### Constraints

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

### Behavior to define

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

The report said an exited terminal would not appear in a pane title. An
exited terminal can still be shown in a pane for review from the terminal
list, so the implementation gives it the unnumbered title `[terminal] <name>`.

### Reproduction

1. Open a terminal (`[terminal #1]`), exit its shell, and close it from the
   terminal list.
2. Repeat several times.
3. Open another terminal while no other terminal is running. Its title showed
   the next counter value, such as `[terminal #5]`, rather than `#1`.

### Documentation

The report asked for updates to the terminal sections of `docs/user-guide.md`
(the Navigator description of the title ID, and the `[terminal #<id>]`
paragraph under Terminals) and the terminal passages of
`context/reference/ui-vocabulary.md`.
