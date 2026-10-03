---
title: "Unsupported CSI functions execute unrelated basic commands"
status: resolved
reported: 2026-10-03
resolved: 2026-10-03
commit: a09e163
---

## Resolution

Commit `a09e163` (`fix(terminal): ignore unsupported CSI intermediate functions`).

Emulator::csi dispatch previously matched the private marker and final byte while generally ignoring intermediates. Since none of its supported CSI functions use intermediates, it now rejects those complete sequences before dispatch. Unsupported editing, cursor, rendition, mode and query functions cannot alias supported commands that share a final byte.

Coverage: unsupported_csi_intermediates_do_not_alias_basic_commands in tests/terminal_sequences.rs verifies unchanged screen text, cursor, colors, alternate-screen state and device replies, followed by ordinary printing. It failed before the fix; all 16 terminal sequence tests pass.

## Report

CSI dispatch ignores intermediate bytes for most commands. An unsupported
sequence such as `ESC [ 2 SP @` is consequently treated as ordinary Insert
Character and shifts the current row. Likewise `ESC [ 2 SP A` moves the cursor
up, although the intermediate byte identifies a distinct control function.
Unsupported sequences sharing a final byte with erase, rendition, mode, and
device-report commands can also change terminal state or generate replies.

The emulator must distinguish the complete control-sequence identifier,
including intermediate bytes. No currently supported CSI command has an
intermediate byte, so these sequences should be ignored without changing
screen cells, cursor position, rendition, modes, or replies. Following ordinary
text and supported CSI sequences must still work.

Reproduce by writing a row, placing the cursor in its middle, and feeding
`ESC [ 2 SP @`. The row should remain unchanged rather than gaining two blank
cells.
