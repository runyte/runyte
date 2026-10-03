---
title: "Cancelled terminal sequences swallow subsequent output"
status: resolved
reported: 2026-10-03
resolved: 2026-10-03
commit: 36addad
---

## Resolution

Commit `36addad` (`fix(terminal): cancel interrupted control sequences consistently`).

Parser::step now handles CAN and SUB before state dispatch and abandons all partial sequence data, including ignored strings. ESC restarts non-string collection and ignore states while string states retain their terminator handling. This prevents printable bytes after cancellation from being interpreted as abandoned sequence finals or payload. With cancellation centralized, the CSI ignore state needs only one conditional byte-range check for normal completion.

Coverage: cancelled_control_sequences_resume_printing_across_chunk_boundaries and escape_restarts_interrupted_control_sequences in tests/terminal_sequences.rs reproduce cancellation and restart from escape, CSI, OSC and ignored string states. Both failed before the fix; all 15 sequence integration tests and all 15 parser unit tests pass.

## Report

The terminal parser does not consistently leave a control sequence when it
receives CAN (`0x18`), SUB (`0x1a`), or a new ESC. In a CSI parameter or
intermediate state, cancellation enters the ignore state, consuming the next
printable final byte. A cancelled DCS remains in its ignored string state and
can hide all following text until another escape arrives. ESC inside an escape
intermediate or CSI intermediate also fails to restart parsing cleanly.

For example, `ESC [ 31 CAN OK` displays `K` rather than `OK`, and
`ESC P ignored CAN OK` loses both letters. After `ESC [ SP ESC [ 2 J`, the
new erase-display sequence should be parsed rather than printed as text.

CAN and SUB should discard the entire partial sequence and resume ground-state
text immediately. A new ESC should restart escape parsing from every non-string
collection or ignore state; string states must retain their existing ESC-backslash
terminator handling. Behavior must be independent of PTY read chunk boundaries.
