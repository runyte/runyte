---
title: "Lines selected with x or X do not paste as whole lines"
status: resolved
reported: 2026-08-09
resolved: 2026-08-09
legacy_commit: 3a570a2
---

## Resolution

Commit 3a570a2 (`Preserve linewise yanks from line selection`) fixed
`execute_editor_command`, which was clearing the transient `x`/`X` line
selection state before `yank_value` classified the register. Yank dispatch
now carries that provenance into the existing linewise register model and
adds the terminating newline when necessary, so `p` inserts the copied rows
as rows. Explicit `v` selections do not carry that provenance and remain
characterwise, including selections extended to a line boundary with `g`
motions.

Coverage is in
`x_and_x_yanks_paste_whole_lines_but_v_yanks_remain_characterwise` in
`src/app.rs`.

## Report

Lines copied with `x` or `X` should paste with `p` as whole lines, inserting a
new line. At the time of the report pasting required opening a line first
(`o p`).

Text copied with `v` should keep its existing behavior: a selection yanked
with `v` and `g` motions pastes inline as following text rather than as a new
line.
