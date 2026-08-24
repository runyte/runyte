---
title: "Last-pane close message does not explain how to quit"
status: resolved
reported: 2026-08-09
resolved: 2026-08-09
legacy_commit: bdffa9b
---

## Resolution

Commit `bdffa9b` ("Clarify last-pane close guidance") corrects the status
message emitted by `App::close_pane` when its last-pane guard rejects a close.
The guard already kept the only pane open, but its lowercase, terse message
did not tell the person how to leave the editor. It now uses the requested
sentence and leaves the existing close behavior unchanged.

`closing_the_last_pane_explains_how_to_quit` in `src/app.rs` covers the exact
message and verifies that the sole pane remains active.

## Report

`:close` (and `Space w c`, `Ctrl-w c`, `Ctrl-w Ctrl-c`) on the last pane
reported "cannot close the last pane". The message should instead read:
"Cannot close the last pane. To quit runyte type :quit".
