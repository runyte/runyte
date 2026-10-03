---
title: "Special input trace files block debug startup"
status: resolved
reported: 2026-10-03
resolved: 2026-10-03
commit: e2bac56
---

## Resolution

Commit `e2bac56` (`fix(debug): validate input trace files before truncation`).

open_input_trace now creates or opens the destination without truncating it first, uses nonblocking Unix opens, and verifies a regular-file descriptor before truncation. Invalid trace destinations return errors without waiting for a FIFO reader. Existing regular-file creation and per-launch truncation remain intact; release builds still omit this diagnostic feature.

Coverage: input_trace_refuses_special_files_without_blocking and its owned input_trace_fixture in src/tui/tests/input_trace.rs cover a FIFO, a symlink to it, absent-file creation, and ordinary truncation. The bounded subprocess timed out before the fix and passes afterward.

The integration fixture `a_second_process_is_refused_when_an_explicit_log_is_owned`
in `tests/diagnostic_log.rs` previously kept the first process alive by blocking
its debug input trace on a FIFO. It now starts a persistent host with the explicit
log destination and verifies a health response before launching the competing
process. The test still checks the log-ownership refusal and that every shared
log record belongs to the original process, and it also runs in release builds.

## Report

Debug builds open the optional `RUNYTE_INPUT_TRACE` destination with a blocking,
truncating `OpenOptions` call. A FIFO without a reader blocks startup before
input processing; other special objects are accepted as trace destinations.

The trace must open only regular files and validate the opened descriptor
before truncation or writes. Opening a FIFO or a symbolic link to one must
return an error promptly without changing the special object. Existing regular
files must still be truncated for a new trace, and missing regular trace files
must still be created. Release builds do not enable this diagnostic trace.
