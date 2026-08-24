---
title: "There was no command to open a new scratch buffer"
status: resolved
reported: 2026-08-11
resolved: 2026-08-11
legacy_commit: 7cef601
---

## Resolution

Commit `7cef601` (`add new scratch buffer command`) added the semantic
`new-buffer` editor command and registered it as the primary `Space b n`
binding. `App::open_scratch_buffer` creates a new pathless `Buffer::scratch`
with a matching empty syntax slot, then uses the existing `switch_buffer`
boundary to retarget only the active pane. That boundary records the previous
buffer in the pane's jumplist and resets the new view consistently with other
buffer switches.

The binding is present in the shared registry, so execution, key hints, and
generated help remain aligned. The user-facing binding tables in `README.md`
and `context/reference/helix-keymap-v1.md` document the addition.

Covered by
`app::tests::space_b_n_opens_a_fresh_scratch_buffer_in_the_current_pane` in
`src/app.rs` and `implemented_and_unsupported_bindings_are_explicit` in
`tests/keymap.rs`.

## Report

Runyte did not have a command for opening a new scratchpad in the current
pane. `Space b n` was requested to display a fresh scratch buffer there.
