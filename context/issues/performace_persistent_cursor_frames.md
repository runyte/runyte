# P2 — Persistent cursor movement publishes complete editor frames

Priority: P2 (normal). The extra work is confirmed in code; an end-to-end
measurement is needed to establish its contribution to perceived latency.

At commit `22fd664`, `publish_attached_frame` in `src/main.rs` prepares a complete
host frame and attempts `TerminalDamageFrame::between` from
`src/protocol/frame.rs`. That delta mechanism handles terminal changes and
rejects changes to ordinary editor-pane state, including cursor changes.
Ordinary cursor movement consequently publishes a complete editor frame,
including unchanged pane rows, through the persistent-session transport.

This adds frame conversion, copying, serialization, local transport, decoding,
and client rendering to cursor-only actions. The 2026-09-28 investigation
verified this path by code inspection but did not measure its latency or wire
volume. Headless snapshot timings do not include this overhead.

To reproduce, compare standalone mode with `runyte -a` using the same release
binary, source fixture, and terminal dimensions. After syntax and services
settle, alternate `h` and `l` without scrolling. Capture host-to-client frame
types and byte counts, and measure input through rendered acknowledgement.
Repeat at 120×40 and 240×80 and with multiple visible panes. Use isolated
temporary workspace/configuration storage and set fixture-owned
`XDG_CONFIG_HOME` in subprocess builders.

Cursor-only changes should avoid retransmitting unchanged document rows.
Candidate work includes editor-row or caret deltas and reuse of unchanged pane
data. The protocol must preserve frame identity, correct base selection,
bounded queues, full-frame recovery, attachment switching, and convergence
when intermediate publications are dropped. A delta must never refer to a
base the client has not received.

Validation should cover cursor and selection changes, edits, scrolling,
geometry changes, dropped frames, slow clients, and detach/reattach, alongside
the latency and byte-count comparison. Shared snapshot reconstruction is
tracked separately in `performace_unchanged_pane_snapshots.md`.
