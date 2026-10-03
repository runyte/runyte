---
title: "Opening a diagnostic log can block on a special file"
status: resolved
reported: 2026-10-03
resolved: 2026-10-03
commit: 8e6b060
---

## Resolution

Commit `8e6b060` (`fix(log): reject special files when opening diagnostic logs`).

`App::open_log_buffer` reopened the logger pathname with `fs::read_to_string`.
A replaced pathname could therefore wait forever for a FIFO writer on the
editor thread. It now uses `path_safety::open_regular_file`, which opens without
blocking on Unix and validates the opened descriptor before reading. A refused
object leaves the active document intact; regular-file symlinks still work.

Coverage: `log_open_refuses_a_replaced_fifo_without_waiting_for_a_writer` and
its bounded `special_log_file_fixture` in `tests/log_buffer.rs` reproduce the
hang, verify document preservation, and read a regular-file symlink. The full
log-buffer integration target passed.

## Report

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
