# Windows restart acceptance can pass without exercising replacement

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

## Suggested fix method

Separate the restrictive-policy refusal case from a required successful-restart
acceptance case. The latter must fail with an actionable infrastructure error
if its runner cannot create the required detached process; it must not return
success or silently skip. Provision a native Windows acceptance environment
with the required process policy, and have CI require the exact acceptance test
to run and pass. Preserve the refusal test under a deliberately restrictive job
where feasible, with owned child processes and cleanup.

The required successful-path test should verify the complete sequence:

- A normal restart confirms the original host's exit and publishes a distinct
  replacement process for the same workspace.
- With unsaved editor state, normal restart refuses and retains the exact
  running publication and protected state.
- `--session-restart --force` replaces that host only after confirmed stop,
  with the documented loss of unsaved state and unchanged saved file contents.
- Cleanup stops every replacement host even when an assertion fails; no
  detached process or publication remains after the fixture.

Keep exact-publication selection, ambiguity and stale-selection refusal,
preflight-before-stop ordering, cancellation ownership and bounded cleanup.
Do not weaken the production process policy or allow fallback to a path, name,
or PID merely to make acceptance pass. All subprocesses must use fixture-owned
configuration and temporary runtime storage.

Run the native `windows_session_cli` target, the new required acceptance case,
the Rust handoff gates and the cross-platform CI gates. Record the exact source
revision, native environment and successful-path result in the Windows delivery
record. If a suitable acceptance environment cannot be provided before release,
keep the gate explicitly outstanding and settle whether to defer the public
restart command rather than treating refusal-only coverage as certification.
