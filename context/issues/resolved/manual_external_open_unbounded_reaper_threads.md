---
title: "Manual external opens create unbounded infallible reaper threads"
status: resolved
reported: 2026-10-03
resolved: 2026-10-03
commit: f9c8f3d
---

## Resolution

Commit `f9c8f3d` (`fix(files): reserve bounded reapers before manual external opens`).

Manual Unix external opens now use the existing system-opener ownership machinery. A shared pool permits 16 outstanding handlers, creates the fallible worker before spawning its child, and retains its slot until reaping completes. Failure to reserve or start a worker cannot launch an unowned child or panic the editor. The existing five-second startup observation deadline reports an uncertain outcome without retrying the external operation. Manual program arguments and the final literal pathname retain their existing parsing.

The gated manual-opener regression also releases its helper during unwinding,
so an assertion failure cannot leave the fixture waiting for its normal
success-path release.

Coverage: manual_opens_share_bounded_admission_and_keep_literal_arguments in src/external_open.rs verifies refusal before spawn, argument identity, slot ownership, and release after reaping. `reaper_thread_admission_failure_never_spawns_and_returns_the_record` in `src/external_open/tests/system.rs` exercises the shared path with an injected worker-start failure. All 27 external_open tests pass, with the owned cache fixture ignored outside its parent.

Known limitation: long-lived external handlers retain admission slots until they exit; reaching the existing 16-handler shared limit now also refuses further manual opens with editor feedback.

## Report

On Unix, manually opening a file or URL starts its external process and then
creates an infallible, dedicated reaper thread. Each application that stays
alive retains another thread, without the bound already used by native Windows
and plugin system-opening paths. If thread creation fails, `thread::spawn`
panics after the child has already started, leaving no assigned reaper owner.

Use the existing bounded external-opener ownership for these launches as well.
Admission and fallible reaper creation must succeed before a child can start.
A full opener pool must produce an actionable native error; an admitted child
must retain its independent stdio and process group and remain owned until it
exits. Thread-creation failure can be injected through the existing opener
fixture instead of exhausting actual machine resources. Explicit program
arguments and the final literal file/URL argument must keep their existing
meaning.
