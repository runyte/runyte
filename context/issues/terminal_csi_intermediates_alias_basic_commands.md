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
