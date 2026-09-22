# Codex cursor flickers above its prompt in a Windows terminal pane

On Windows 11, run Codex CLI 0.155.1 inside a Runyte standalone integrated
terminal hosted by Windows Terminal and Windows PowerShell. After opening
`/status`, type in the Codex prompt or leave the prompt idle. A caret matching
Runyte's terminal caret appears intermittently near the left edge of the pane,
above the Codex input line, while the input-line caret also appears to blink.
The active terminal's row and column indicators in Runyte's global status line
change rapidly even without input. This visual instability is absent when an
ordinary text-buffer pane has focus. The misplaced blinking cursor was also
reported when Codex runs directly in Windows Terminal, outside Runyte.

The expected behavior is a stable visible caret at the Codex input position and
a stable Runyte status line while the child is idle. The terminal pane should
still display a child cursor for ordinary shells and other terminal programs.

Runyte paints the active terminal caret from the current emulator position in
`TerminalSession::view` (`src/terminal/mod.rs`) and `terminal_line`
(`src/ui.rs`). The status snapshot (`src/snapshot.rs`) also reports that live
terminal position as its row and column. The standalone frontend drains PTY
chunks and publishes pending frames on a 16 ms tick (`src/main.rs`). These
paths expose any intermediate child cursor position received during a redraw;
they do not establish where the Codex prompt is. A native ConPTY fixture probe
on this host confirmed that a child's `CSI ?25l` cursor-hide sequence reaches
Runyte's PTY output, so cursor-hide loss was not reproduced by that probe.

The [upstream Codex cursor-ordering report](https://github.com/openai/codex/issues/39710)
describes a closely matching Windows failure. Its post-ConPTY capture showed
frames ending with the cursor visible on a status or transcript row before a
later write moved it to the composer. The
[`custom_terminal.rs` source tagged 0.155.1](https://github.com/openai/codex/blob/rust-v0.155.1/codex-rs/tui/src/custom_terminal.rs)
positions the cursor before showing it, but flushes the screen diff before
hiding the cursor. A visible cursor can therefore move across changed cells
during the diff. The current
[`custom_terminal.rs` on Codex `main`](https://github.com/openai/codex/blob/main/codex-rs/tui/src/custom_terminal.rs)
hides a visible cursor before flushing nonempty updates, then positions it and
shows it. This source comparison offers a likely diagnosis for 0.155.1; the
installed binary was not captured at the PTY boundary. The reproduction
outside Runyte supports an upstream cause for the misplaced caret, while
Runyte's live cursor and status rendering makes the effect visible in its pane.

To resolve or narrow this issue, reproduce with a Codex build known to hide
the cursor before flushing a nonempty screen diff, then capture the bounded
ConPTY output around a
misplaced-caret frame if it persists. Compare the emulator cursor position and
visibility at each published Runyte frame with the child's final cursor state.
Any Runyte mitigation must keep ordinary shell cursors and prompt editing
responsive; simply suppressing all terminal cursors would hide useful state.
