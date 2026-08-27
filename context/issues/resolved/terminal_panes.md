---
title: "A pane cannot run an interactive program"
status: resolved
reported: 2026-08-20
resolved: 2026-08-20
legacy_commit: 3189a44
---

## Resolution

Commit `3189a44` (`Run a program in a pane`) added `src/terminal/`, a
pseudoterminal subsystem with the same discipline as `src/git/` and
`src/lsp/`: one owner, bounded state, and no other module aware that an
escape sequence exists. `:terminal` runs a program in the active pane,
`Space t n` is the same command on a key, and five further commands under
`Space t` list, leave, close, freeze, and send to a session.

The type question came first, because everything else depends on it. A
terminal is a pane content type, not a twenty-third `BufferKind`.
`Pane::terminal` names the live session a pane shows *instead of* its
buffer, and `buffer` keeps its ordinary meaning as the document that pane
returns to. Nothing in the new module is a rope, a transaction, or an undo
group: a transaction log of `htop` redraws is unbounded and settles no
question anyone asks, escape sequences overwrite arbitrary cells rather
than editing at an offset, and under the alternate screen there is no
linear text to be a document of. A `BufferKind` variant would have had to
answer "not applicable" at roughly a hundred and forty match sites, which
is what a wrong type looks like from the inside.

The module splits along what can be tested without a child process.
`grid.rs` is the styled cell rectangle plus a bounded scrollback, and knows
nothing about panes or drawing. `parser.rs` is the byte state machine; it
emits actions and has never heard of a grid, which is what lets it be
driven by escape-sequence fixtures. `emulator.rs` turns those actions into
screen changes and owns the modes a child switches on. `pty.rs` is the only
place in Runyte that forks a process onto a tty. `keys.rs` encodes a
`KeyStroke` back into the bytes a child expects. The emulator is Runyte's
own rather than a borrowed crate, in keeping with the project's own fuzzy
scorer, picker, walker, and diff.

Key routing is the decision the rest of the behaviour follows from. In
Insert mode every key goes to the child, `Escape`, `Ctrl-c`, `Ctrl-w`,
`Ctrl-o` and `Space` included, because a terminal that swallowed any of
them could not run `vim`, could not interrupt anything, and could not be
used as a shell. That is routed in `App::handle_key_stroke` ahead of the
registry rather than expressed as a `BindingScope`, because the requirement
is to disable the keymap wholesale and a scope can only narrow or add. A
terminal pane still reports `BindingScope::Global`, so
`normal_and_select_bind_the_same_sequences` and the shadowing invariants in
`src/keymap.rs` keep covering it unchanged. Reading the pane's buffer scope
instead would have given a pane showing a shell the bindings of whatever
explorer or Git view it was going to go back to, which is the one wrong
answer available there.

`Ctrl-\` leaves Insert mode, and `is_terminal_leader` accepts two
spellings of it. A terminal implementing the kitty keyboard protocol
reports the character; a legacy one has only the control byte `0x1c`,
which Crossterm decodes as `Ctrl-4` from the historical table. Runyte
requests the enhanced protocol but cannot require it — `src/main.rs`
disables it on macOS outright — so a hatch bound to one spelling alone
would be unreachable on the terminals that need it most. This was found by
driving the real binary on a pty rather than by any unit test, all of which
passed with one spelling. Pressing the key from Normal mode sends a literal
`Ctrl-\` and returns to Insert, so twice is how the key itself is sent.

Normal mode navigates and copies but does not edit, which is what the
report asked for and also all that is deliverable. Motions scroll the
session's history, `y` copies its output to the system clipboard, `p` sends
the clipboard to the child. `App::execute_terminal_command` refuses
everything editing-shaped by *category* rather than by name, so a command
added later is refused by default: the failure that matters is an editing
command silently reaching the document behind the pane, which is a real
file on disk. Project-wide search, windows, splits, pickers, Git, help, and
every colon command are unaffected. Buffer-local search is refused by name,
since its category also holds the project-wide commands that should work.

Two commands connect a terminal to the rest of the editor, and between them
they deliver what the request was actually after. `Space t y` freezes a
session's screen and history into an ordinary read-only generated buffer,
where search, multiple selections, `n`/`N`, and yank all work on real text.
`Space t s` sends a buffer's selection — or the whole buffer when nothing is
selected — to a terminal as one bracketed paste. Its target is the single
terminal visible in another pane, else the one shown most recently, else the
only one open.

Sessions outlive the pane showing them: leaving a terminal, opening a file
in that pane, and closing the split all leave the child running, and
`Space t t` lists every session with its directory and state. `Pane::retarget`
clears the terminal unconditionally, including when retargeting to the
buffer already named, because asking for a document is asking to stop
looking at a terminal. `App::split` clears it on the new pane, since one pty
has one size and two panes would fight over every resize. Terminals are
deliberately absent from `Space b b`, because they are not buffers.

`App::prepare_view` is the one place that knows a pane's new shape, so it is
where `TIOCSWINSZ` is sent. `EditorSnapshot` reports the child's cursor and
line count for a terminal pane rather than the hidden buffer's, which would
otherwise have said `[RO]` about a shell and put the caret at 1:1 while it
was somewhere else. A wheel over a terminal moves its history and a click
only focuses the pane, because the cells under the pointer belong to the
child and have no offsets to place a caret in. `quit_allowed` refuses once
while any child is running and `:q!` goes through, which is the same bargain
the dirty-buffer guard makes: a long-running program is exactly the reason
someone opened a terminal.

`SnapshotRow` cannot carry a terminal, so `PaneSnapshot` gained a styled
cell rectangle beside its rows, mirrored into `src/protocol/frame.rs`. That
is a deliberate hole in the rule that the host ships semantics and the
client resolves colour. A `TextRun` names a tree-sitter scope because the
editor knows what a run of text *is*; a child process on a pty has only ever
said what colour it wants, so there is nothing semantic left to send. Only
`Color::Default` is left to the frontend, which is what keeps a shell
readable in a light theme instead of assuming a black screen.

Deliberate deviations from what the report asked for. The report wanted
Normal-mode search and selection over the displayed cells; that is offered as
an explicit command producing an ordinary buffer instead, because a selection
over a live grid that repaints under it is a selection over nothing. The
report's suggested shape had a compose buffer shipped first as a separate
step; it is a command inside this work instead, which delivers the same value
without a second surface. `i`, `a`, `I`, `A`, `o`, and `O` all simply type
again rather than opening a line or moving first, because there is no offset
to open a line at.

A later review found six defects, fixed in the same series. Three were
resource bounds. The output channel was unbounded, so a child writing faster
than the editor applied could queue 64 KiB buffers without limit even though
the grid's scrollback was bounded; it is now a bounded channel, and the reader
thread's `blocking_send` is what turns a full queue into backpressure on the
child — the pty buffer fills and the child's next `write` blocks in the
kernel. The event loop drained with `while let Ok(..) = try_recv()`, which
never terminates against a producer that refills faster than the loop empties,
so `yes` in a pane would have starved rendering and input outright; the drain
is now `terminal::drain`, capped at one queue's worth, which is enough to
coalesce a repaint and cannot run away. And the wire protocol still advertised
version 19 after `PaneSnapshot` gained a required field, so a newer client
attaching to an older host passed the handshake and then failed to
deserialize its first frame; `protocol::VERSION` is 20.

The other three were correctness. A scrolled-back view measured its position
as a distance from the live screen, so every line the child printed moved the
window forward through the text being read; `Grid::retired` is a monotonic
count of lines that have left the top — which the scrollback *length* stops
reporting once the limit is reached and lines start being dropped from the
front as fast as they join the back — and `feed` grows the scroll offset by
that delta, clamped to what is still kept. Erase, `DCH`, `ICH`, and a
narrowing resize could each leave one half of a double-width character behind;
since the renderer skips width-0 cells, an orphaned spacer silently shortened
the row and shifted everything after it, so `blank_span`, `split_before`, and
`clear_trailing_lead` now carry the same invariant `write` always had.
Finally, `encode_paste` translated every line feed to a carriage return even
inside the paste brackets. Outside them that is right — bare text goes to a
line discipline, where only a carriage return is Enter — but between them the
payload is data that a program reading raw input can tell apart, so a
multi-line selection sent to a TUI editor arrived as carriage returns instead
of the line breaks it had. Bracketed payloads now cross unchanged.

A later macOS CI run exposed one more lifetime defect. Dropping
`TerminalSessions` let Rust drop the session map before the output registry,
so a child that had filled its bounded queue could still have a PTY reader
blocked on that registration while the child was being killed and reaped.
Linux happened to complete that teardown; macOS could leave the test waiting
on the deliberately endless child. `TerminalSessions::drop` now uses the same
`close_all` ordering as an explicit shutdown: remove and wake every output
registration first, then drop the PTYs. The runaway-child integration test
keeps its queue saturated through that owner-drop boundary.

Covered by `tests/terminal.rs`, which runs real children on real
pseudoterminals through the ordinary input and frame paths:
`a_child_runs_in_the_pane_and_its_output_is_drawn`,
`opening_a_terminal_starts_in_insert_mode_and_the_exit_is_one_way`,
`typing_in_insert_mode_reaches_the_child`,
`escape_is_sent_to_the_child_rather_than_leaving_insert_mode`,
`a_terminal_pane_refuses_commands_that_would_edit_the_buffer_behind_it`,
`normal_mode_scrolls_the_scrollback_and_returns_to_the_live_screen`,
`the_pane_is_named_by_the_title_the_child_sets`,
`a_finished_child_leaves_its_last_screen_readable`,
`leaving_a_terminal_shows_the_buffer_again_without_ending_the_child`,
`closing_a_terminal_ends_its_child_and_forgets_it`,
`copying_a_terminals_output_opens_it_as_an_ordinary_buffer`,
`a_selection_composed_in_a_buffer_can_be_sent_to_a_terminal`, and
`quitting_refuses_a_running_terminal_once`. Every child there is a fixed
program — `cat`, `echo`, `printf`, `sh` — rather than `$SHELL`, so no test
reads the person's rc files. The parser and emulator are table-driven against
escape-sequence fixtures in `src/terminal/parser.rs` and
`src/terminal/emulator.rs`; the grid's invariants, including wrap, wide
characters, scrollback, and resize, are in `src/terminal/grid.rs`; key
encoding is in `src/terminal/keys.rs`; and `src/terminal/pty.rs` checks that a
child sees the size it was given and that input reaches it.

The review fixes are covered by
`a_child_that_never_stops_writing_cannot_starve_the_editor` in
`tests/terminal.rs`, which floods a real pane and asserts both that every
drain stays inside the bound and that the editor still answers a keystroke
afterwards; by `draining_stops_at_one_queue_however_much_the_child_writes` and
`the_output_queue_is_bounded_so_a_child_cannot_grow_it_without_limit` in
`src/terminal/mod.rs`, the first driven by a producer that never stops, so an
unbounded drain would hang the test rather than fail it; by
`a_scrolled_back_view_holds_still_while_the_child_keeps_printing`,
`a_view_following_the_live_screen_keeps_following_it`, and
`a_held_view_clamps_to_what_the_scrollback_still_keeps` in the same file; by
`erasing_part_of_a_double_width_character_takes_the_whole_of_it`,
`shifting_across_a_double_width_character_leaves_no_orphan`,
`narrowing_never_keeps_half_a_character`, and
`retirement_keeps_counting_after_the_scrollback_limit` in
`src/terminal/grid.rs`; by `bracketed_paste_keeps_the_line_endings_it_was_given`
in `src/terminal/keys.rs`; and by
`protocol_version_and_request_bounds_are_explicit` in `src/protocol/mod.rs`.

The subsequent integrated-terminal work moved `App`, terminal sessions, and
their ptys into the persistent workspace host and added bounded terminal frame
damage, so detach, workspace switching, and reattachment now preserve a live
child. That removes the original host-ownership limitation recorded below
without changing the original implementation commit named in the frontmatter.

A later review found four more defects. `Parser::osc` bounded the sum of OSC
payload bytes but not the vector of empty fields; CSI subparameters and escape
intermediates had the same structural hole. The parser now charges both stored
payload and structural fields to one sequence budget. `TerminalSession::search_review`
converted every regex match by recounting characters from the start, making a
dense search quadratic, while `ensure_review` repeatedly recounted the growing
snapshot once per line. Snapshot construction now carries its character count
forward, matching advances one byte/character cursor through the ordered,
non-overlapping results, and review text, indices, and match storage all count
toward the workspace terminal budget. `App::quit_allowed` applied
the standalone live-child refusal inside persistent hosts even though quitting
there only detaches; that guard is now standalone-only. Finally, frontend hint
observation saw Terminal Insert before `App::handle_key_stroke` changed
`Ctrl-w` to Normal mode, so the command executed but its window hints never
appeared. `App::key_hint_mode_for_key` classifies that transition key by the
Normal-mode namespace it enters while leaving every other Insert key hidden.

A later mode-consistency change made both terminal escapes one-way and shared
them with ordinary Insert mode. `Ctrl-\` (including its legacy `Ctrl-4`
spelling) and `Ctrl-w` now resolve through the declarative Insert keymap to
`enter-normal-mode`; neither can return a Normal terminal to input. A second
`Ctrl-w` begins the ordinary window namespace, and `i` is the direct return to
terminal input. The literal-control-w compatibility command stays Normal after
sending byte `0x17`.

The subsequent window-namespace and review work refined that transition again.
`Ctrl-w` now begins its restricted window namespace directly, directional
continuations leave input in live Normal while moving, and `Ctrl-\` stages
Insert → live Normal → review across two presses. Neither spelling returns a
Normal terminal to input; `i` remains the direct return.

A macOS report exposed a fifth portability defect in `Pty::spawn`: its
hand-written setup sized a newly opened primary/master before the replica
existed, then opened that replica only after `setsid` without `O_NOCTTY` and
asked for it as the controlling terminal again. Darwin surfaced `ENOTTY` from
that non-portable sequence. `open_pair` now uses the platform `openpty`
contract to open both endpoints with the initial size on the slave before the
child makes it controlling stdin, stdout, and stderr. Resizes after startup
continue through the master.

A later macOS CI run exposed a portability error in the real-PTY test fixtures,
not in the terminal event ordering. Several tests launched `echo`, `printf`, or
a finite shell as the child, waited for its output, and then continued testing
live terminal behavior. Runyte correctly removes a terminal as soon as its exit
event is applied, while the output queue correctly keeps retained bytes ahead
of that event. The tests therefore depended on their assertion running in the
narrow interval between those two events. Their deterministic child programs
now print the same prelude and continue on `cat`, so tests for drawing, colour
queries, review, titles, scrollback, and generated output operate on a genuinely
live terminal. The tests that exercise process exit still use finite children
and drain through the real lifecycle boundary.

A later saturated macOS run exposed the same scheduling sensitivity in the
low-level finite-child fixture itself: `/bin/echo` could close the slave before
the background PTY reader received a timeslice. The fixture now uses a finite
shell that writes the same output and waits for one input line. The test sends
that line only after observing the output, then verifies the exit event. This
keeps the output-and-exit contract without depending on the reader winning an
immediate-exit scheduling race.

These later fixes are covered by
`empty_osc_fields_are_counted_toward_the_sequence_limit`,
`csi_subparameters_are_counted_toward_the_sequence_limit`, and
`escape_intermediates_are_counted_toward_the_sequence_limit` in
`src/terminal/parser.rs`; `dense_review_search_maps_matches_in_one_forward_pass`
and `review_memory_accounting_includes_search_matches` in
`src/terminal/mod.rs`; `persistent_quit_detaches_while_a_terminal_keeps_running`
and `control_w_exits_then_opens_the_window_namespace_and_sends_the_literal_byte`
in `tests/terminal.rs`;
`terminal_control_w_leaves_insert_without_starting_a_window_prefix` in
`tests/key_hints.rs`; and `a_child_sees_the_size_the_pty_was_opened_with` plus
`input_reaches_the_child` in `src/terminal/pty.rs`.

A later terminal-program compatibility report corrected one deliberately
missing query in `Emulator::osc`. Runyte rendered a child's default cells with the
current editor foreground and background, but discarded the read-only OSC 10
and OSC 11 requests through which a child discovers those same defaults.
Codex therefore fell back to dark semantic colours even when its unpainted
cells sat on Runyte's light background. `TerminalSessions` now keeps the
resolved default RGB pair in sync with startup, theme preview, cancellation,
and saving; every emulator answers exact read-only queries through its
existing bounded reply path. A `reset` colour remains unanswered because its
outer-terminal value is genuinely unknown. Setters, palette queries, and OSC
52 remain ignored.

That correction is covered by
`default_colour_queries_answer_with_the_current_theme_colours` and
`default_colour_queries_do_not_expand_into_palette_or_clipboard_access` in
`src/terminal/emulator.rs`,
`changed_default_colours_reach_existing_session_emulators` in
`src/terminal/mod.rs`, `theme_names_activate_the_matching_theme` and
`focused_theme_setting_previews_without_remembering_and_saves_on_enter` in
`src/app.rs`, and `a_child_can_discover_the_effective_default_background` in
`tests/terminal.rs`.

Further known limitations. Terminals are Unix only: Windows needs ConPTY,
which is a second implementation of the hardest part, and
`context/issues/windows_support.md` already records that Runyte disables a
feature there rather than shipping an unsound one. Inline images — kitty
graphics, sixel — are not passed through. Resizing does not reflow wrapped
lines, because emulators disagree about what a resized wrapped line should
become and a wrong guess corrupts a live full-screen program worse than a
truncated one. `OSC 52`, colour setters, and palette queries are ignored on
purpose: a child should not write the person's clipboard unasked or change its
terminal's palette. Read-only OSC 10/11 queries expose only the default colours
Runyte is already using to render the child. Scrollback is bounded at five
thousand lines and is never written to disk, since terminal history routinely
contains credentials, private commands, and unrestricted program output.

## Report

A pane should be able to run an interactive program the way a terminal
emulator does: a shell, `htop`, `vim`, a pager, or another full-screen TUI. A
terminal is a second kind of pane content rather than another `BufferKind`.
Two constraints follow from that boundary:

- A terminal is not a document. It has no rope, no transaction log, no undo
  stack, no saved text, and no disk state, because none of those answer a
  question anyone asks of `htop`. It therefore lives in its own module beside
  `src/git/` and `src/lsp/`, with the same discipline: one owner, bounded
  state, and no other module aware of an escape sequence.
- A terminal grid cannot be edited with multiple cursors. The child process
  owns its input and Runyte sees only the cells it painted. A multi-cursor edit
  over those cells would edit a picture of the text rather than the child's
  state. Normal mode over a live terminal therefore navigates and copies; it
  does not edit. Structured input can instead be composed in an ordinary
  buffer and sent to the terminal as one paste.

Required behavior:

`:terminal` runs a program in the active pane, `$SHELL` with no argument and a
shell-split command line with one. The pane's buffer stays where it is and is
what the pane returns to. Sessions outlive the pane showing them: leaving a
terminal, opening a file in that pane, or closing the split all leave the
program running, and a list reaches it again. Terminals do not appear in the
buffer picker, because they are not buffers.

In Insert mode every key belongs to the child, including `Escape`, `Ctrl-c`,
`Ctrl-w`, `Ctrl-o`, and `Space`. The escape hatch cannot be `Escape`, because
`vim` and `htop` inside the pane need it, so it is a dedicated key no
plausible child wants.

In Normal mode the scrollback is navigable, the output is yankable, and the
clipboard can be sent to the child. Commands that would edit must be refused
rather than applied to the document waiting behind the pane, which is a real
file.

Two commands connect a terminal to the rest of the editor. One freezes a
session's screen and history into an ordinary read-only buffer, where search,
multiple selections, and yank work on real text. The other sends a buffer's
selection — or the whole buffer — to a terminal as a single bracketed paste.

Scope and platform:

Unix only. Windows needs ConPTY, which is a second implementation of the
hardest part; `context/issues/windows_support.md` already records that Runyte
disables a feature there rather than shipping an unsound one.

The emulator is Runyte's own, in keeping with the project's preference for
owning its core. It needs what an interactive program on a pty actually uses:
the 256-colour palette and true colour, the usual attributes, scroll regions,
insert and delete, the alternate screen, bracketed paste, application cursor
keys, cursor-position reports, and window titles.

Scrollback is bounded and is not written to disk because terminal history may
contain credentials, private commands, and unrestricted program output.

The initial report left PTY ownership in persistent mode undecided. Keeping a
terminal alive across client detach or workspace switching requires the host
to own the PTY and the frame protocol to carry a styled cell grid with bounded
damage.

Also left undecided: forwarding mouse reporting to a child that wants the
pointer, inline image display, whether sessions survive a host restart, and
ferrying a client's clipboard to a host-side child so that image paste works
when the two are different machines.

After the integrated terminal shipped on macOS, bare `:terminal` failed before
starting the configured zsh process. The interaction line reported:

```
:terminal (Run a shell or command in this pane . failed: cannot start zsh: Inappropriate ioctl for device (os error 25))
```
