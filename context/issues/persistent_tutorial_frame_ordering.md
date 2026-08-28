# Persistent tutorial integration test assumes input-correlated frames

GitHub Actions run
[`33183416669`](https://github.com/runyte/runyte/actions/runs/33183416669)
failed on commit `0cd1583ac0244f290a01c980c08dab7f85476bd8` in the Linux
`Gates (ubuntu-latest)` job. Formatting, Clippy, the macOS suite, the
performance budgets, the Ubuntu 22.04 release build, and the Rust 1.88 check
all passed. The only failing test was:

```text
tutorial_persistent_lesson_completes_across_a_real_client_reattachment
```

It failed at `tests/persistent_host.rs:548` immediately after entering
`:tutorial sessions`:

```text
assertion failed: frame_text(&tutorial).contains("PERSISTENT SESSIONS")
```

The test sends each input through `send_input`, which receives and returns the
next host response. It then treats the response following Enter as the frame
caused by that Enter. Interactive visual responses do not have that causal
contract. `ResponseSender` keeps frames and terminal damage in a replaceable
visual slot, and the host may publish asynchronous frames independently of an
input request. Under scheduler contention, a frame queued before the Enter can
therefore be read first even though the tutorial transition is applied
correctly. The same assumption appears after reattachment, where the first
frame after `Welcome` is asserted to contain `NEXT STEPS`.

The persistent tutorial state transition itself has direct coverage in
`src/app/tests/tutorial.rs::persistent_tutorial_finishes_only_after_detach_and_reattach`.
The complete macOS test job passed in the failing run, and the exact integration
test passes when run in isolation, which is consistent with a timing-dependent
test failure rather than incorrect tutorial state.

## Expected behavior

The integration test waits for the observable editor state it exercises. After
Enter, one complete frame must contain both `PERSISTENT SESSIONS` and
`persistent tutorial token`. After reattachment, one complete frame must
contain both `NEXT STEPS` and the same token. An older queued visual response
must not be attributed to the most recent input.

The wait must be bounded by an absolute state deadline in addition to the
per-response host deadline. A timeout should identify the transition being
awaited and retain the last complete frame's identity and text for diagnosis.
Unexpected semantic responses such as refusal, detach, shutdown, or protocol
error must fail immediately rather than being swallowed.

## Constraints

Production frame ordering should not be changed to satisfy the test. Visual
frames are intentionally asynchronous and replaceable so a slow client can
converge on a current complete snapshot without retaining every intermediate
frame. Making a response correlate with each input would require a broader
protocol acknowledgement or request-identity design.

A CI retry or an unbounded receive loop is not sufficient. A retry conceals the
race, while a receive loop can wait forever if the desired state is never
published. The test helper must inspect the response already returned by
`send_input`, because that response may already be the matching frame. When it
sees terminal damage, a non-matching complete frame, or a quiet visual stream,
it can request `ClientRequest::Resynchronize` to obtain current complete state.

The fix belongs at the integration-test boundary in `tests/persistent_host.rs`.
No GitHub Actions workflow change is required.

## Reproduction

Run the complete locked suite on a loaded Linux machine so the persistent-host
integration test competes with the other test binaries:

```sh
cargo test --locked
```

The failure is intermittent. The recorded CI occurrence is in job
[`98890141040`](https://github.com/runyte/runyte/actions/runs/33183416669/job/98890141040).
When it occurs, `cargo test` exits with code 101 after 17 of the 18
`tests/persistent_host.rs` cases pass. Running only the named test commonly
passes:

```sh
cargo test --locked --test persistent_host \
  tutorial_persistent_lesson_completes_across_a_real_client_reattachment \
  -- --exact
```
