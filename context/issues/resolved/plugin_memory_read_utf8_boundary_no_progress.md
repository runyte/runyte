---
title: "Memory provider UTF-8 reads can acknowledge empty non-final chunks"
status: resolved
reported: 2026-10-03
resolved: 2026-10-03
commit: e42d1c6
---

## Resolution

Commit `e42d1c6` (`fix(examples): reject memory resource reads without UTF-8 progress`).

The memory example now stops its UTF-8 end-boundary adjustment at the requested offset and rejects a chunk that cannot contain the next scalar before actual EOF. Interior offsets still fail decoding; valid aligned reads and a true empty EOF response retain their byte coordinates. This matches the remote-provider example contract and avoids non-progressing read loops.

Coverage: the resource read assertions in docs/plugins/check_applications.py exercise too-small limits and interior offsets in both conditional and confirmed best-effort modes, then verify aligned Unicode reads and EOF. The new public-wire assertions failed before the fix; all 11 application conformance tests pass.

## Report

The shipped memory-provider example can acknowledge a resource read with an
empty string and `eof: false` when the requested byte limit cannot hold the
next UTF-8 scalar. Its end-boundary adjustment also walks backward past the
requested offset when that offset itself splits a scalar. The resulting empty
slice can conceal the invalid starting offset.

For a resource containing `é猫`, reads at offset 0 with limit 1, offset 2 with
limit 2, or offset 1 with limit 1 return empty non-final chunks. These reads
must return `invalid_argument`, preserving the requested byte coordinates.
Valid scalar-aligned reads must still return complete characters, and an empty
read at the actual end of the resource must remain a successful EOF response.

The transport-neutral remote provider already enforces these boundaries; the
memory example should demonstrate the same public resource-read contract in
both conditional and confirmed best-effort modes.
