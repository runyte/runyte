# `Space t s` falls back to the terminal last shown, not the one last focused

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
