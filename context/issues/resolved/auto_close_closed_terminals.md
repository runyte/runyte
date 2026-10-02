---
title: "Exited terminals remain in terminal and Finder lists"
status: resolved
reported: 2026-10-02
resolved: 2026-10-02
commit: 7351dda
---

## Resolution

Commit `7351dda` (`Forget exited terminal sessions by default`) resolves this
issue. `App::finish_terminal` previously detached an exited terminal from its
pane but always retained the session and output in `TerminalSessions`, so
terminal-list and Finder discovery continued to include it.

The `editor.auto_close_terminal` setting defaults to `true` and is available
in `Space o o`. After preserving the existing pane restoration, exit status,
and diagnostic logging, the exit handler removes the terminal session and its
focus-history entry. Setting the option to `false` retains future exited
sessions for review and search. Changing the setting does not retroactively
discard already retained sessions, including during settings preview.

The open terminal list updates through its existing identity-aware refresh.
Finder removal uses the existing paced dirty-terminal refresh: content rows
retire through the bounded content scan, while name rows remap both local and
background-worker resource indices. This preserves a surviving selected result
when an unrelated hidden terminal exits, including when a scan is in flight.

Regression coverage:

- `exiting_the_last_terminal_reveals_its_buffer_without_quitting_runyte` and
  `exiting_a_terminal_preserves_its_pane_when_another_pane_exists` in
  `tests/terminal.rs` exercise real child exits and default removal.
- `terminal_exit_removes_open_manager_row_and_preserves_other_terminal_input`
  in `src/app/tests/session_navigation.rs` covers hidden-terminal removal from
  an open terminal list without changing the active terminal's input mode.
- `exited_terminals_are_forgotten_from_open_finder_names_and_contents` and
  `background_finder_keeps_selected_surviving_result_when_a_terminal_exits`
  in `src/app/tests/search_and_pickers.rs` cover name/content removal,
  nonzero exit status, and selected-result preservation with a real scanner.
- `auto_close_terminal_setting_previews_rolls_back_persists_and_reloads` in
  `src/app/tests/config_reload.rs` covers configuration preview, cancellation,
  persistence, and reload. Existing retained-output, review, terminal-list,
  Finder, and idle-retirement tests explicitly disable automatic closure.
- `terminal_exit_refreshes_its_stale_background_file_with_either_retention_setting`
  in `src/app/tests/navigation_and_files.rs` covers refreshing the file revealed
  by an exit, both retention settings, and preservation of the exit status.

## Report

Exited terminal sessions remain in `Space t t` and Finder results. By
default, exiting a terminal (for example, by typing `exit`) should forget
the terminal session and remove it from both lists. The retention behavior
should be configurable through `Space o o`.

To reproduce, start a terminal, type `exit`, then open `Space t t` or the
Finder. The exited terminal is still listed.
