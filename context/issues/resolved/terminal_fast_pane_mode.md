---
title: "Terminal directional pane keys could expose review instead of live Normal"
status: resolved
reported: 2026-08-23
resolved: 2026-08-23
legacy_commit: 7dca32c
---

## Resolution

Commit `7dca32c` (`Separate terminal normal and review transitions`) fixed two
mode transitions that had been conflated. `App::execute_terminal_command`
handled every terminal `EnterNormalMode` command by calling
`TerminalSession::begin_review`, so the first `Ctrl-\` from Insert captured a
snapshot even though the editor mode had only just changed to Normal. It now
handles Terminal Insert separately: the first press stops routing keys to the
child while keeping its output live, and the next `Ctrl-\` captures review.
Review motions still capture on demand from live Normal, and `i` still returns
to terminal input.

`App::focus_from_terminal_insert` also used the generic Insert window-command
cleanup after directional focus. That cleanup intentionally preserved Insert
on a terminal destination, which meant directional focus did not establish one
consistent live-Normal boundary. Directional focus now leaves terminal input
before moving, clears stale review on a terminal destination, and remains in
Normal. Both `Ctrl-w h/j/k/l` and configured `Ctrl-h/j/k/l` already resolve to
the same four command identities, so the correction covers both spellings
without adding a dispatch exception or a second keymap.

Coverage lives in `tests/terminal.rs`: test
`control_backslash_steps_from_terminal_input_through_normal_to_review` covers
both `Ctrl-\` and legacy `Ctrl-4`; test
`directional_pane_keys_focus_another_terminal_in_insert` covers all four
directions with prefixed and fast keys; tests
`control_w_focus_moves_directly_from_terminal_insert_without_sending_input`
and `fast_pane_keys_move_out_of_terminal_input_without_reaching_the_child`
verify that neither spelling sends input or captures review when leaving for a
document pane.

A later verification against a persistent session exposed a deployment gap in
the original fix. The host owns `App`, its keymap, and input dispatch, but the
change had left the private protocol at version 27. A newly built client could
therefore attach to a still-running pre-fix host and continue to observe the
old behavior. Protocol version 28 now makes this host-semantic change an
explicit incompatibility, following the existing precedent for keymap changes;
a new client refuses the stale host instead of silently delegating input to
it. `protocol::tests::protocol_version_and_request_bounds_are_explicit` in
`src/protocol/mod.rs` pins the new version, while the transport compatibility
tests cover rejection of the preceding version.

Known limitation: `Ctrl-w w` retains its existing Insert-mode terminal
destination behavior. This report concerned only directional `h/j/k/l`
focus, and the next-window behavior remains documented separately.

## Report

Terminal mode transitions behaved inconsistently between the fast
`Ctrl-h/j/k/l` pane motions and the prefixed `Ctrl-w h/j/k/l` motions. The fast
motions could appear to put a terminal into review mode instead of immediately
leaving terminal input in Normal and moving to the pane in the requested
direction.

Both spellings were expected to behave identically. From Terminal Insert, one
directional command was expected to enter Normal and move immediately, without
a second press and without capturing terminal review.

`Ctrl-\` was expected to retain a distinct staged transition:

```text
Insert -> Normal -> review
```

The first press was expected to leave terminal input in live Normal, and the
second to enter review.
