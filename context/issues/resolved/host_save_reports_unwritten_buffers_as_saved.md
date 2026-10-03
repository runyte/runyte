---
title: "Host saves report success without writing the buffer"
status: resolved
reported: 2026-10-03
resolved: 2026-10-03
commit: e79a6b2
---

## Resolution

Commit `e79a6b2` (`fix(session): report host saves only after completed writes`).

The native save workflow deliberately reports failures through editor feedback while returning a handled command. Its new explicit SaveDisposition separates completed writes from refusals, failures, and deferred work. The host adapter now returns an error for NotWritten and refuses commit-message and directory operations before starting asynchronous work or confirmation. Native command behavior and successful writes with durability warnings remain intact.

Coverage: host_save_returns_errors_for_failed_and_stale_writes, host_save_refuses_read_only_commit_and_directory_buffers_without_side_effects, and host_save_returns_the_revision_after_writing_the_requested_buffer in src/workspace/host/tests/save_results.rs. All three pass; failed writes and refused special buffers reproduced before the fix.

## Report

`WorkspaceHost::save_buffer` reports a successful buffer revision when the native
save workflow refuses the write or encounters an I/O error. Native saves report
these outcomes through editor feedback and normally return `Ok(())`; the host
adapter currently interprets that command-handling result as a completed write.

Reproduce by opening a file through a host, modifying its buffer, overwriting the
file externally, and calling `save_buffer`. The external contents remain intact
and the editor reports the stale-file conflict, but the host returns success.
A scratch buffer with no path and a clean generated read-only buffer also return
success without being written. Saving a commit-message buffer or an edited
directory can initiate asynchronous work or a native confirmation while the
caller immediately receives success.

The synchronous host API should return a revision only after an ordinary file
write completes. Refused and failed writes must return errors, and saves that
require asynchronous execution or native confirmation must be refused without
starting them. Native save-command feedback and confirmation behavior should
remain unchanged. A write that commits successfully with a durability warning
must retain its explicit successful-write outcome rather than being inferred
from feedback severity or the buffer's dirty flag.
