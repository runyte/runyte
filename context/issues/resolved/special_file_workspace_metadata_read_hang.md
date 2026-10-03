---
title: "Persistent-session metadata reads can block on special files"
status: resolved
reported: 2026-10-03
resolved: 2026-10-03
commit: 46fa60c
---

## Resolution

Commit `46fa60c` (`fix(session): reject special files in endpoint records`)
changes `workspace::transport::read_bounded_file` to use the shared
nonblocking, descriptor-checked regular-file opener. Owner and permission
checks alone accepted an owner-private FIFO, and the byte limit could not
bound its blocking open. The existing privacy checks and byte limits remain
in place around the regular-file read.

`special_file_endpoint_reads_refuse_without_blocking` and its owned subprocess
fixture `special_file_endpoint_fixture` in `src/workspace/transport.rs` cover
metadata and stored-name FIFOs, device refusal, absent records, and normal
stored-name writes. The regression failed at its ten-second deadline before
the fix and passed in 0.01 seconds after it. All 49 transport tests passed;
the helper fixture is intentionally ignored outside its parent. The parent
reviewer checked the shared read call and the fixture independently.

Known limitation: this rejects special objects; regular-file reads can still
wait for a stalled underlying filesystem.

## Report

A Unix persistent session can hang while reading endpoint metadata or its stored
session name if the corresponding runtime file is replaced by a named pipe.
The endpoint permission checks accept an owner-private FIFO with mode `0600`,
and `read_bounded_file` opens it with blocking `File::open`. Without a writer,
that open never completes. The read byte limit does not bound the open.

Endpoint discovery, liveness checks, and stored session name loading should
reject non-regular files promptly while retaining the existing private-file
checks, size limits, and normal metadata/name behavior. The opened descriptor
must be checked so a file replacement between the path check and open cannot
turn the read into an unbounded FIFO wait.

Reproduction: in a temporary workspace, create the normal private endpoint
directory and replace `endpoint.json` (the path returned by
`LocalEndpoint::metadata`) with a mode-`0600` FIFO. Calling the endpoint's
recorded-host liveness check blocks indefinitely. A FIFO at the stored session
name path produces the same result when loading the name. A bounded subprocess
fixture can exercise both paths without leaving a blocked test thread behind.
