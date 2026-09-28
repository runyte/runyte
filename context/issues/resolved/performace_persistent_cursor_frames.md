---
title: "Persistent cursor movement publishes complete editor frames"
status: resolved
reported: 2026-09-28
resolved: 2026-09-28
commit: 351b979
---

## Resolution

Commit `351b979` (`Send editor row deltas for persistent cursor frames`)
resolves the redundant wire publication. `publish_attached_frame` prepared a
complete host frame and tried only `TerminalDamageFrame::between`; that function
rejects editor-pane changes, so every ordinary cursor movement sent all visible
document rows again. The same full-frame behavior existed in the Windows host.

`EditorDamageFrame` now carries the previous frame ID, new frame ID, status,
changed visible rows, and changed pane cursor positions. It is used only while
pane structure, viewport, active document revision, mode, geometry, and other
non-row presentation state are compatible. Edits, scrolling, geometry changes,
and broad repaints use a complete frame. The publisher also sends a complete
frame while an earlier visual write remains pending, so a replacement cannot
refer to a base the client has not received. The client folds consecutive
deltas into its latest complete frame before replacing the renderer's queued
visual. A stale or invalid base requests a complete-frame resynchronization;
the host clears its retained base for that peer. Both Unix and Windows bundled
client paths handle the new protocol variant.

The release comparison in `context/reference/startup-performance.md` recorded
median serialized cursor-publication sizes falling from 18,777 to 1,309 bytes
at 120×40 with one pane and from 68,672 to 1,348 bytes at 240×80 with two
panes. Six input-to-TestBackend-draw samples per case had overlapping timing
ranges, so the comparison establishes the wire reduction without establishing
a terminal-perceived latency improvement.

Regression coverage: `src/protocol/frame.rs` tests
`editor_damage_round_trips_cursor_selection_and_status_from_exact_base`,
`editor_damage_requires_full_frame_for_edit_scroll_geometry_and_broad_repaint`,
and `changed_one_row_pane_does_not_retransmit_large_unchanged_sibling`;
`src/workspace/transport.rs` tests
`replacing_an_in_flight_visual_keeps_the_new_one_pending` and
`reader_coalesces_editor_damage_chain_while_renderer_is_stalled`;
`tests/local_protocol.rs` tests
`persistent_cursor_movement_sends_rows_only_and_reattach_starts_with_full_frame`.

Known limitation: The latency fixture ends after a Ratatui `TestBackend` draw;
it does not measure a real terminal write, PTY flush, or rendered
acknowledgement, and it does not compare against standalone mode. Windows
protocol paths were updated but Windows-specific tests were not executed on
Linux.

## Report

Priority: P2 (normal). The extra work was confirmed in code; an end-to-end
measurement was needed to establish its contribution to perceived latency.

At commit `22fd664`, `publish_attached_frame` in `src/main.rs` prepared a
complete host frame and attempted `TerminalDamageFrame::between` from
`src/protocol/frame.rs`. That delta mechanism handled terminal changes and
rejected changes to ordinary editor-pane state, including cursor changes.
Ordinary cursor movement consequently published a complete editor frame,
including unchanged pane rows, through the persistent-session transport.

This added frame conversion, copying, serialization, local transport,
decoding, and client rendering to cursor-only actions. The 2026-09-28
investigation verified this path by code inspection but did not measure its
latency or wire volume. Headless snapshot timings did not include this
overhead.

The proposed reproduction called for comparing standalone mode with
`runyte -a` using the same release binary, source fixture, and terminal
dimensions. After syntax and services settled, it would alternate `h` and
`l` without scrolling, capture host-to-client frame types and byte counts,
and measure input through rendered acknowledgement. The requested cases were
120×40, 240×80, and multiple visible panes, with isolated temporary
workspace/configuration storage and fixture-owned `XDG_CONFIG_HOME` in
subprocess builders.

Expected behavior was that cursor-only changes avoided retransmitting
unchanged document rows. Proposed approaches included editor-row or caret
deltas and reuse of unchanged pane data. The protocol needed to preserve
frame identity, correct base selection, bounded queues, full-frame recovery,
attachment switching, and convergence when intermediate publications were
dropped. A delta could never refer to a base the client had not received.

Requested validation covered cursor and selection changes, edits, scrolling,
geometry changes, dropped frames, slow clients, and detach/reattach, alongside
the latency and byte-count comparison. Shared snapshot reconstruction is
tracked separately in `performace_unchanged_pane_snapshots.md`.
