---
title: "Windows restart acceptance can pass without exercising replacement"
status: resolved
reported: 2026-09-25
resolved: 2026-09-25
commit: 5295d2e
---

## Resolution

Commit `5295d2e` (Require Windows restart acceptance and characterize save
visibility) makes successful replacement an explicit native CI gate. The branch
already separated refusal and success into owned subprocess fixtures. Acceptance
now checks exact dirty contents and revision after refusal, reopens the file in
the forced replacement to prove unsaved state was discarded, and injects an
assertion panic to exercise replacement cleanup. The innermost breakaway job
permits detached creation inside an outer kill-on-close owner. Unsupported
runner policy fails with a diagnostic; refusal cannot count as success.

Native execution also exposed a race in `shutdown_request`: the host could consume
the request and close its pipe before the sender's final flush completed,
producing Windows error 232. Native pipe-closure errors now retain the reader to
recover a response without resending. Explicit refusals and unrelated failures
remain errors. Closure yields only a stop receipt; `await_host_stopped` still
requires the authenticated, pinned original process to exit before replacement.
Production process policy and exact-publication selection are unchanged.

At source revision `5295d2e`, Windows 11 Home build 26200,
`x86_64-pc-windows-msvc`, Rust 1.97.1 passed formatting, denied-warning all-target
Clippy, the full `cargo test --locked` suite with two test threads, and all three
exact CI acceptance commands. The session CLI target passed 12 tests; its two
ignored helpers execute inside their owners. The exact restart success gate
passed in 0.84s. Subagent review had no actionable findings.

Regression coverage:

- `restart_success_replaces_host_and_preserves_protected_state`,
  `restart_refuses_detached_policy_without_breakaway_job`, and
  `restart_refuses_ambiguous_live_id_prefix` in `tests/windows_session_cli.rs`.
- `shutdown_recovers_response_after_native_flush_closure_without_hiding_refusal`,
  `shutdown_timeout_has_no_receipt_and_eof_is_not_proof_of_process_exit`, and
  `supported_stop_receipt_observes_only_the_original_process_exit` in
  `src/workspace/windows_lifecycle/tests.rs`.

Known limitation: remote cross-platform CI for this fix is pending. Local native
acceptance is established; the configured CI gate is not represented as a
completed remote result. The Windows delivery record tracks remote validation.

## Report

The 2026-09-25 release review at `b39bc86` found that the enabled Windows
`--session-restart [WORKSPACE]` command lacks unambiguous acceptance evidence for
its successful replacement path. This is a validation gap, not an observed
failure of successful restart.

In `tests/windows_session_cli.rs`,
`restart_preflights_detached_policy_and_replaces_only_confirmed_real_host`
starts a real host and invokes `--session-restart`. If the command fails while
the fixture is running in a Windows job, the test accepts this specific error:

```text
detached host preflight was denied
CreateProcessW could not create the detached inheritance parent
(os error 5)
```

It verifies that the original host remains alive and its ready publication is
unchanged, then returns successfully. This exercises an important refusal
contract, but skips the subsequent assertions for successful stop-to-start
replacement, refusal with unsaved editor state, and replacement with `--force`.

The guarded-restart checkpoint in
`context/reviews/windows_phase2_handoff.md` records that local validation took
this refusal branch and explicitly leaves successful-path acceptance pending.
The reviewed green
[CI run 36069941965](https://github.com/runyte/runyte/actions/runs/36069941965/job/107868292438)
reports the combined test as `ok` without identifying which branch executed.
That output cannot establish that the skipped behaviors were exercised.

Expected release evidence must independently prove both safe refusal under a
restrictive process policy and successful restart on a native Windows runner
that permits detached creation. A green test that proves refusal must not also
stand in for acceptance of replacement and protected-state handling.

### Required acceptance

The restrictive-policy refusal case and required successful-restart acceptance
case must be separate. The latter must fail with an actionable infrastructure
error
if its runner cannot create the required detached process; it must not return
success or silently skip. A native Windows acceptance environment must provide
the required process policy, and CI must require the exact acceptance test to
run and pass. The refusal test belongs in a deliberately restrictive job where
feasible, with owned child processes and cleanup.

The required successful-path test should verify the complete sequence:

- A normal restart confirms the original host's exit and publishes a distinct
  replacement process for the same workspace.
- With unsaved editor state, normal restart refuses and retains the exact
  running publication and protected state.
- `--session-restart --force` replaces that host only after confirmed stop,
  with the documented loss of unsaved state and unchanged saved file contents.
- Cleanup stops every replacement host even when an assertion fails; no
  detached process or publication remains after the fixture.

Exact-publication selection, ambiguity and stale-selection refusal,
preflight-before-stop ordering, cancellation ownership and bounded cleanup must
remain intact. Production process policy must not be weakened, and fallback to
a path, name or PID solely to make acceptance pass is not allowed. All
subprocesses must use fixture-owned configuration and temporary runtime storage.

Required validation includes the native `windows_session_cli` target, the new
required acceptance case, the Rust handoff gates and cross-platform CI gates.
The Windows delivery record must identify the exact source revision, native
environment and successful-path result. Without a suitable acceptance environment
before release, the gate remains explicitly outstanding and public restart
deferral requires a decision; refusal-only coverage is not certification.
