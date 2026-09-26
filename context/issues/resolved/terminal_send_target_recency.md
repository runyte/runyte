---
title: "Space t s falls back to the terminal last shown, not the one last focused"
status: resolved
reported: 2026-09-26
resolved: 2026-09-26
commit: 1c2f78f
---

## Resolution

Fixed in `1c2f78f` (Send to the terminal focused last and never implicitly to
an exited one).

`send_target_terminal` in `src/app/terminal_workflows.rs` fell back to
`App::last_terminal`, a single `Option<TerminalId>` assigned wherever a
terminal was moved into a pane — `show_terminal`, `uncover_terminal`, and
jump navigation in `src/app/editing.rs` — whether or not that pane held focus.
Moving focus onto a pane that already showed a terminal never touched it, so
with two terminals on screen the target was whichever had been placed last.
None of the three rules checked whether the program was still running, so an
exited terminal could be chosen and then refused.

`last_terminal` is replaced by `focused_terminals`, a recency list of terminal
identities with the most recent last. It is updated in two places only:
`activate_pane` in `src/app/file_workflows.rs`, the single path through which
`Ctrl-w`/`Space w` movement, pointer presses, `swap-window`, closing the active
pane, and jump or session restoration change the active pane; and
`move_terminal_to_pane`, when the pane receiving the terminal is the active
one, which covers `Space t t`, the Navigator, `:terminal`, and uncovering or
jumping in the active pane. A list rather than a single slot is what lets rule
2 pass over an exited terminal to the next most recently focused live one.
Closing a terminal removes it from the list.

`send_target_terminal` now filters every rule by `TerminalSession::live`:
the only live terminal visible in another pane, then the most recently
focused live terminal, then the only live terminal in the editor. The
`that terminal's program has exited` refusal is reached only through an
explicit `:terminal-send ID` or `:terminal-send NAME`. `Space t y`
(`copy_terminal_output`) uses the same recency list but accepts exited
sessions, since their retained output can still be copied.

Uncovering a terminal in a pane that is not active no longer affects the
fallback: the terminal was shown, not focused, which is the distinction the
report draws.

The keymap register entry for `Space t s` in
`context/reference/helix-keymap-v1.md` now states the three rules.

Tests, in `src/app/tests/commands.rs`:

- `sending_without_a_target_follows_the_terminal_focused_last` follows the
  reproduction with three panes and also checks that an exited terminal on
  screen leaves the other visible live one as the rule 1 choice.
- `sending_without_a_target_skips_a_focused_terminal_that_has_exited` checks
  that rule 2 skips an exited, most recently focused terminal and that naming
  it explicitly is still refused.
- `sending_buffer_text_chooses_one_terminal_and_names_why_it_cannot` continues
  to cover the unchanged refusals.

## Report

`Space t s` (`:terminal-send` without an argument) sends the selection, or the
whole buffer when nothing is selected, to a terminal as one bracketed paste.
`send_target_terminal` in `src/app/terminal_workflows.rs` chooses the target:

1. The only terminal visible in a pane other than the active one.
2. Otherwise `last_terminal`, the terminal most recently *shown* in a pane.
3. Otherwise the only terminal in the editor.
4. Otherwise nothing: `no terminal to send to`.

`last_terminal` is set only when a terminal is moved into a pane: by
`show_terminal` (`Space t t`, the Navigator), by uncovering a pane, and by jump
navigation. Focusing a pane that already shows a terminal, entering Terminal
Insert there, or typing into it does not update it.

With two terminals visible side by side, rule 1 does not apply, and the target
is whichever of the two was most recently brought into its pane. That can
differ from the terminal worked in last, and nothing on screen indicates which
one will receive the text. The status message `sent N characters to NAME`
reports the choice only after the paste has been delivered.

## Expected behavior

Rule 2 chooses the terminal that was most recently **focused**: the terminal
shown in the pane that most recently held focus, the same notion of recency
`Space w x` (`swap-window`) uses through `previously_focused_pane`.

A terminal counts as focused when its pane becomes the active pane by any
route: `Ctrl-w` or `Space w` pane movement, a pointer press, or showing the
terminal in the active pane. Showing a terminal in the active pane already
focuses it, so the cases that update `last_terminal` today remain covered.

Terminals whose program has exited are never chosen without an explicit
target. Every rule considers only live terminals: rule 1 looks for the only
live terminal visible in another pane, rule 2 takes the most recently focused
live terminal, and rule 3 the only live terminal in the editor. An exited
terminal that was focused last is skipped in favor of the next most recently
focused live one.

Today an exited terminal can be chosen and then refused with
`that terminal's program has exited`. That refusal remains only for an explicit
`:terminal-send ID` or `:terminal-send NAME`.

Rule 4 and the other refusals are unchanged: sending to the terminal in the
active pane, empty text, and pastes over 1 MiB or into a full input queue.

## Reproduction

1. Open a document pane and split twice, so there are three panes.
2. In the second pane run `:terminal` (terminal A); in the third run
   `:terminal` (terminal B). B is now the terminal most recently shown.
3. Focus terminal A's pane and type something into it.
4. Focus the document pane, select some text, and press `Space t s`.

Observed: the text is sent to terminal B. Expected: terminal A, the terminal
focused most recently.
