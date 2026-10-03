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
