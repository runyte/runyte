Opening `:log-open` blocks the editor when the installed logger's destination
has been replaced externally with a named pipe that has no writer. The logger
retains its original open descriptor, but the command reopens the pathname with
`fs::read_to_string` on the editor thread. A device file can likewise block or
produce an unbounded stream instead of a finite log document.

The log page should read a regular file and report a read failure for special
objects without changing the active document. Regular log files and symlinks
to regular files should remain readable. The opened descriptor must determine
the file type so a pathname check cannot race with the open.

Reproduce on Unix by installing a file logger in temporary storage, removing
its pathname, replacing that path with `mkfifo`, and executing `:log-open`
without connecting a pipe writer. A bounded subprocess should verify that the
command completes with an error and preserves the active document.
