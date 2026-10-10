---
title: "Rapid command input can dismiss a document preview"
status: resolved
reported: 2026-10-10
resolved: 2026-10-10
commit: 941abd3
---

## Resolution

`941abd3` (`Order preview input against acknowledged editor frames`) corrects
`NativeView` routing against a frame that predates an earlier physical key.
Previously `preview_key` could consume a command's `h` as local navigation
before the editor's command-mode frame arrived, then dismiss the capture on
`s`. The frontend now waits for a correlated post-input frame before routing
further physical input while a preview is retained. Standalone frames
acknowledge processed input; private protocol 78 carries the same correlation
for persistent attachments. Host acknowledgment uses normal complete-frame
publication, so finder refills cannot expose partial lists. Attached wheel
batches flush before acknowledgment requests, including ignored-input paths.

The bounded FIFO includes keys, text commits and pointer actions, preserving
their original attachment, presented frame and repeat kind. It coalesces
adjacent compatible drags and bounds queued text at 8 MiB. A later frame
cannot grant delayed input authority to approve a prompt it never saw.
Pointer actions with stale geometry use the host's existing stale-frame
validation. Window-local font and clipboard shortcuts remain native actions.

Later Linux CI exposed a throughput problem in the first correction: even
source-pane and command input waited for a GPUI render after each host frame.
A controlled slowdown left the save pending at the fixture's fixed check; the
correct complete text appeared 2.20 seconds later. Acknowledged host-owned
input now drains on frame arrival, without waiting for GPU rendering. Every
new frame invalidates preview readiness, so newly active preview input still
waits for `prepare_previews`. Saved-text and undo assertions wait for their
actual completed file state, and retain exact content checks.

Coverage includes all seven tests in
`crates/runyte-native/src/tests/input_routing.rs`: command burst correlation,
attachment changes, queue bounds, mixed text/pointer ordering, text memory
accounting, drag coalescing across input boundaries, and acknowledged input
waiting for preview preparation.
`terminal_edition_host_accepts_preview_from_a_window_attachment` in
`tests/persistent_host.rs` checks correlated command-mode, normal-mode and
ignored-key responses. `tests/native_window.py --document-preview`, both with
and without `--mux`, sends an undelayed `:hsplit` burst and checks retained
preview navigation. Workspace transitions wait for expected pixels; zoom
reset still requires exact raster equality after bounded renderer completion.

## Report

With a document preview open, a rapid `:hsplit plain.txt` can lose its first
letter and dismiss the preview. Linux Xvfb/lavapipe acceptance observed a dark
source pane after `Ctrl-Up`; the expected preview raster remained absent even
after ten seconds. The same failure occurred before helper integration.

A diagnostic key trace records `:` and `s` routed against the same painted
frame. The intervening `h` is consumed as preview navigation before command
mode reaches the frontend, and `s` triggers preview dismissal. Sending the
command as an undelayed X11 key burst reproduces it; constraining the editor,
renderer and fixture to one CPU also exposes the ordinary 25 ms typing case.

Expected behavior: the command leader and remaining command text reach the
editor in order, the horizontal split succeeds, and returning focus retains
preview navigation. Both standalone and persistent windows must behave this
way. Any queued input must preserve the attachment and frame physically seen
at capture time so delayed Enter cannot approve a newly presented prompt.
