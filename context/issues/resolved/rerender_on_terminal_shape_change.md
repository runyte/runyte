---
title: "The editor does not redraw when the terminal changes shape"
status: resolved
reported: 2026-08-14
resolved: 2026-08-14
legacy_commit: 780d9f6
---

## Resolution

Fixed by commit 780d9f6, "Redraw the standalone editor when the terminal
changes shape".

The standalone event loop in `src/main.rs` treated every terminal event as a
candidate for editor input before anything else. It called
`convert_event`, and `convert_event` deliberately returns `Ok(None)` for the
lifecycle events that are not input — `FocusGained`, `FocusLost`, and
`Resize`. The `let ... else` arm handling that `None` recorded the event with
the key-repeat detector and then executed `continue`, which jumped past the
`terminal.draw` call at the bottom of the loop body. A resize therefore
reached the process, was correctly recognized as not-input, and then produced
no frame at all. The old shape stayed on screen until some later event — a
motion, a command, or the 250 ms Git refresh tick — went through the loop and
redrew, which is exactly the set of triggers the report names.

Nothing about the resize needed to be applied to editor state. Ratatui
reconciles its back buffers with the real terminal size inside `draw`, and
the layout already reads its geometry from `frame.area()` on every frame, so
the entire fix is to let the loop reach the draw it was skipping. Resize now
matches its own arm ahead of the input arm, guarded by a new
`is_redraw_only_event` predicate, and falls through to the shared draw. Focus
changes keep the quiet path: they do not change the shape, so redrawing for
them would be pointless work. The predicate matches each variant explicitly
rather than using a wildcard, so a future Crossterm event has to be
classified rather than silently defaulting to "no redraw".

Periodic re-rendering was not needed and was not added. The terminal already
delivers `Resize` through the same `EventStream` the loop selects on; the
event was being received and then discarded, so a timer would only have
papered over the discard at the cost of waking an idle editor.

The attached workspace-host path was already correct and is unchanged. In
`run_attached` a resize is matched before conversion, sent to the host as
`ClientRequest::Resize`, and the host's handler updates the client geometry
and sets `changed = true`, which produces a fresh frame. That asymmetry
between the two loops is what made the bug visible only in the standalone
editor.

Covered by `a_resize_carries_no_input_but_still_redraws` in `src/main.rs`,
which pins both halves of the behavior: `convert_event` yields no input for a
resize, so the redraw predicate is the only thing keeping the frame from going
stale, and that predicate accepts a resize while rejecting focus changes and
ordinary keys.

Known limitation: a resize drag that emits a burst of `Resize` events draws
once per event rather than coalescing to the final size. This matches how the
loop already handles a burst of keystrokes and was not worth a debounce, which
would add latency to the common single-resize case.

## Report

Runyte is typically used inside tmux, where the pane size changes often. The
editor does not automatically re-render on the new shape; some action is
required before the new size is reflected.

The actions observed to trigger the re-render are:

- motion
- any command
- or git auto-refresh

Whether periodic re-rendering is the only way to implement automatic
re-rendering on a shape change was left open.
