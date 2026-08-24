---
title: "Terminal pane keys were blocked by a read-only backing buffer"
status: resolved
reported: 2026-08-24
resolved: 2026-08-24
commit: 6780692
---

## Resolution

Commit `6780692` (`Fix terminal pane keys over read-only buffers`) corrected
`App::handle_editor_input`. Its Insert-mode read-only guard inspected the
active pane's backing buffer even while a terminal was in front of that
buffer. `App::handle_key_stroke` had correctly reserved `Ctrl-w` and configured
`Ctrl-h/j/k/l` from the child, but the read-only guard then changed the editor
to Normal and returned before the terminal-scoped pane command reached the
grammar.

The guard now applies only when no terminal is active. Ordinary input remains
owned by the terminal-specific gate, so only Runyte-owned terminal commands can
reach the grammar while the backing buffer is hidden. Read-only protection is
unchanged for panes showing the buffer itself.

Coverage lives in `tests/terminal.rs`:
`pane_keys_dispatch_over_a_read_only_buffer_hidden_by_the_terminal` opens the
read-only About page, places a terminal in front of it, and verifies both
`Ctrl-w h/l` and configured `Ctrl-h/l` movement into and out of the terminal
without an intermediate mode change or review capture.

## Report

Directional pane navigation did not complete in the first key sequence from a
terminal in Insert mode. With `editor.fast_pane_keys` enabled, each of
`Ctrl-h`, `Ctrl-j`, `Ctrl-k`, and `Ctrl-l` changed the editor from Insert to
Normal on the first press, requiring a second press to move. With the prefixed
form, `Ctrl-w` first changed Insert to Normal and the following `h`, `j`, `k`,
or `l` entered terminal review instead of moving to the adjacent pane.

Both forms were expected to execute directional focus immediately. A terminal
destination should start live Insert, while a document destination should
start Normal. Neither form should capture terminal review or send its control
bytes to the child.

The problem was initially reported on macOS and appeared associated with
persistent mode. Commit `31b13b9` requested the keyboard protocol's
disambiguation-only profile on macOS while retaining legacy repeat detection,
but did not change the behavior. A probe under tmux 3.6a confirmed that
Crossterm reported `Ctrl-h/j/k/l/w` as the expected lowercase `Char` key with
exactly the `CONTROL` modifier, followed by a plain `h` for the prefixed form.

A later reproduction isolated the actual boundary: bare standalone `runyte`
failed after opening a terminal over its read-only `[about]` startup page,
while `runyte .` worked because its directory buffer was editable. The defect
therefore applied on any platform to a terminal hiding a read-only buffer,
including About, help, Git views, and generated pages. The macOS enhanced-key
reporting safeguard remains unchanged.
