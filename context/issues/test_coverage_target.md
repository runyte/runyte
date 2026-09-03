# Raise measured test coverage above 95%

Linux and macOS are Runyte's first-class platforms, and the test suite is the
main evidence that both stay correct as the editor changes.

`context/reference/test-coverage.md` records the current baselines. On
`x86_64-unknown-linux-gnu`, `cargo-llvm-cov` 0.9.0 and Rust 1.97.1 cover 92,357
of 101,629 lines, 8,606 of 9,378 functions, and 143,119 of 158,476 regions. The
latest `aarch64-apple-darwin` measurement covers 90,481 of 100,380 lines, 8,450
of 9,253 functions, and 140,408 of 156,605 regions. CI publishes the per-file
summary, retains an HTML report, and fails below an enforced 86% line floor.

The target is above 95% line coverage, with the CI floor and the README badge
raised to match.

## Progress

Commit `26deb27` (`Expand behavior-driven test coverage`) made the target apply
to the total **Lines** percentage printed by the canonical coverage command.
This reported figure remains affected by inline `#[cfg(test)]` code, but it is
the measure the pinned stable tool can enforce directly and consistently. A
custom source parser was rejected as a second, compiler-unverified
implementation of Rust item and configuration semantics. The command,
decision, and platform baselines are recorded in
`context/reference/test-coverage.md`.

That commit added standalone behavior coverage for every protocol input key,
media key, modifier key, pointer identity, literal text, and frame-geometry
round trip. Application-level tests now also cover project-finder navigation
and editing, both file-split directions, scalar-prompt editing, filesystem-plan
review navigation, horizontal pointer scrolling, valid asynchronous Git blame
requests, and asynchronous branch-projection creation and reuse.

On `x86_64-unknown-linux-gnu` with Rust 1.97.1 and `cargo-llvm-cov` 0.9.0,
covered lines increased by 385 without changing the 96,014-line denominator:
85,264 of 96,014 lines (88.80%) became 85,649 of 96,014 (89.20%). Covered
regions increased by 603. `protocol/input.rs` reached 100% line coverage;
`app/input.rs` gained 163 covered lines and `app/git_workflows.rs` gained 58.

A later behavior-focused pass on the same Linux toolchain raised a clean
same-tree baseline from 89,267 of 99,906 lines (89.35%) to 90,301 of 100,134
lines (90.18%), a gain of 1,034 covered lines and 0.83 percentage points after
the larger denominator is accounted for. The retained tests cover guarded Git
staging and branch deletion, mutation outcomes and refusals, LSP request wire
shapes and transient cleanup, picker editing and navigation, workspace event
generation gates, protocol-frame rejection, terminal-parser recovery,
persistent-workspace PTY behavior, and process-termination reporting.
Coverage-only command and provider sweeps found during review were removed.

GitHub Actions run 153 measured the same tree on `aarch64-apple-darwin` after
verifying the runner target. It covered 90,481 of 100,380 lines (90.14%), 8,450
of 9,253 functions (91.32%), and 140,408 of 156,605 regions (89.66%). Both
first-class targets now exceed 90% line coverage.

Commit `f363b02` (`Expand coverage of failure and lifecycle behavior`) records
a continuation pass on Linux that began from a fresh same-tree measurement of
90,308 of 100,134 lines (90.19%), 8,438 of 9,234 functions (91.38%), and
140,184 of 156,240 regions (89.72%). The clean canonical result after the pass
is 90,593 of 100,194 lines (90.42%), 8,449 of 9,238 functions (91.46%), and
140,633 of 156,351 regions (89.95%). Covered lines rose by 285, the denominator
rose by 60, and uncovered lines fell by 225, for a 0.23 percentage-point line
gain.

The retained tests exercise distinct observable behavior: Git cancellation
distinguishes discardable reads, uncertain mutations, completed work, and an
idle service; unborn branches refuse pull and rebase; pushes without upstreams
require one unambiguous default remote; failed language-server launches remain
queryable; real JSON-RPC error envelopes fail only their request; diagnostic
clearing and actionable-message filtering reach editor events; malformed
workspace edits receive an explicit wire refusal; repeated handshakes leave a
control connection usable; wait buffers move from partial to completed; and
standalone completion and command-path hints expose acceptance and entry types.

Review corrected misleading LSP fixtures, replaced fixed-delay assertions with
event and wire-message barriers, isolated the failed-launch executable under a
private temporary directory, strengthened the file-versus-directory rendering
assertions, and removed an existing Git test that only enumerated outcome
variants. Unreachable UI code discovered during review was retained because
removing production code was not necessary to establish the added behavior.

Commit `e960ad8` (`Expand async and refusal coverage`) records the following
Linux continuation. It began from a clean same-tree measurement of 90,579 of
100,194 lines (90.40%), 8,449 of 9,238 functions (91.46%), and 140,616 of
156,351 regions (89.94%). The clean canonical result after the pass is 90,688
of 100,194 lines (90.51%), 8,456 of 9,238 functions (91.53%), and 140,743 of
156,351 regions (90.02%). The unchanged denominator and 109 newly covered lines
produce a 0.11 percentage-point line gain; covered functions rose by seven and
covered regions by 127.

The retained tests exercise asynchronous Git log paging, file-diff creation,
and one-shot stash-list creation through the service boundary; retirement and
recovery after a mismatched LSP response; starting-server and relative-path LSP
refusals before a request reaches the wire; stale reload confirmation after a
host-side buffer close; malformed non-numeric Git history counts; and
cross-request wait-buffer isolation with both waits preserved after refusal.

Quality and coverage reviews removed a synthetic repeated Git-service
attachment test that added no production coverage, routed the reload race
through the reachable host-close boundary, synchronized negative LSP wire
assertions with later wire messages, checked their response tokens, asserted
the expected initial Git discovery operation, and narrowed overbroad test
names. The end-to-end wait-ownership test remains even though it adds no line
coverage: LLVM already credits the shared `ensure!` line through its successful
condition, while the new test uniquely proves the false condition preserves
both requests and leaves the connection usable.

A further pass on Linux began from a fresh same-tree measurement of 92,124 of
101,629 lines (90.65%), 8,589 of 9,378 functions (91.59%), and 142,834 of
158,476 regions (90.13%); the tree had moved on since the recorded 2026-09-02
Linux result, so that fresh run is the comparison. The clean canonical result
after the pass is 92,357 of 101,629 lines (90.88%), 8,606 of 9,378 functions
(91.77%), and 143,119 of 158,476 regions (90.31%). Every added test lives in a
directory named `tests`, so the instrumented total did not move: 233 newly
covered lines, 17 functions and 285 regions produce a 0.23 percentage-point
line gain.

The retained tests cover Markdown's lexical delimiter fallback for pairs that
open and close with the same character, escaped delimiters included; the
insert-mode `delete-to-line-end` command across several carets; the path
popup's overlay snapshots, its Ctrl-c dismissal, a refusing system clipboard,
and a copy into a named register; report scrolling and result-list paging at
both ends; the word each completion kind is displayed with; an asynchronous Git
discard that stops at a refused path and reports what it had already restored;
a worktree started from a remote branch several local branches track; every
named delimiter pair's own inside and around commands; the command palette's
line editing and suggestion movement; and the editing of a typed branch-switch
confirmation. Review corrected the refused-clipboard test, which had asserted
on the transient action feedback a refusal does not set rather than on the
status and retained notification it does.

The largest remaining Linux gaps by uncovered lines are
`app/git_workflows.rs` (815), `main.rs` (727), `app/input.rs` (691),
`git/cli.rs` (412), `ui.rs` (352), `app/language_workflows.rs` (347),
`workspace/transport.rs` (340), `workspace/catalog.rs` (317),
`syntax/mod.rs` (294), and `lsp/mod.rs` (289).

Two shapes recur across the two largest. Twenty-eight of the thirty-two
`git_service.is_some()` guards in `app/git_workflows.rs` have an uncovered
branch, because the application tests mostly drive Git through the synchronous
provider; those guards account for roughly 170 of that file's 815 uncovered
lines rather than most of them. In `main.rs` the mass is concentrated in `run`
(140 uncovered lines), `run_host_server` (118) and `run_attached` (91), with
the attachment and wait-recovery helpers around them. Those are process entry
points and event loops that need a spawned binary or a live attachment; the
file already has its own inline test module, so the gap is not that nothing
tests it.

The issue remains open. Linux still has 9,272 uncovered lines and the latest
macOS measurement has 9,899. The CI floor and README badge remain at 86%,
leaving 4.14 percentage points of headroom below the lower measured platform.
There is no post-change macOS result to justify a higher cross-platform floor,
and a 90% floor would leave only 0.14 percentage points of headroom on the
current macOS measurement. The above-95% target remains open.

## Current macOS baseline

The standard GitHub-hosted `macos-latest` runner used by run 153 was Apple
Silicon, so CI produced the required `aarch64-apple-darwin` baseline without a
larger or self-hosted runner. The existing macOS test job still runs the
ordinary suite without instrumentation.

The baseline was collected by a temporary, non-gating `coverage-macos` job
that used `macos-latest`, set `TMPDIR=/private/tmp`, and followed the pinned
Linux coverage recipe. The job verified the target before measuring so a
change to GitHub's moving runner alias could not silently record an Intel
baseline:

```sh
test "$(rustc -vV | sed -n 's/^host: //p')" = aarch64-apple-darwin
cargo llvm-cov --locked --workspace --no-report
cargo llvm-cov report | tee coverage-summary-macos.txt
```

It installed `cargo-llvm-cov` 0.9.0, used a distinct macOS coverage cache key,
published the summary in the job summary, and retained the HTML report and text
summary as `rust-coverage-macos-arm64`. It did not enforce a new floor. Run
153's toolchain, target, and covered and total counts are recorded in
`context/reference/test-coverage.md`. The temporary job was removed after that
successful measurement rather than doubling the full macOS suite on every CI
run; this recipe remains the way to refresh the baseline when needed.

## What the number means

The recorded baseline counts some inline `#[cfg(test)]` modules in both the
covered count and the total, because stable Rust cannot mark every one of them
as excluded. The reported percentage therefore moves when the ratio of inline
test code to production code moves, without any behavior becoming better tested.
Standalone files under `tests/` are already excluded.

Commit `26deb27` settled the measure: the target applies to the total **Lines**
percentage printed by the canonical `cargo llvm-cov` command, not to a custom
production-only figure. The inline-test caveat remains important when reading
the result because a 95% floor on a measure that test code inflates is weaker
than the number suggests.

Platform-conditional code is the second reason the measure needs stating. A run
on one target cannot execute the other platform's branches, while both remain in
the instrumented total; the current floor is deliberately below the observed
baseline for exactly this reason. Work toward 95% should measure on both
`x86_64-unknown-linux-gnu` and `aarch64-apple-darwin` and record both, and the
enforced floor must be one that holds on whichever target CI measures.

## Expected approach

- Rank modules from the per-file summary by uncovered lines rather than by
  percentage, so the largest absolute gaps are addressed first.
- Add tests at the behavior boundary being changed, as the existing suite does:
  the headless facade, snapshots, the keymap registry, the protocol DTOs, and the
  module-level integration tests under `tests/`. A test that calls a function only
  to execute its lines raises the number without raising confidence.
- Prefer the paths that are currently least exercised: error and refusal
  branches, cancellation and timeout, malformed external output, and the
  platform-specific arms of `cfg` blocks.
- Do not change production code solely to shrink the coverage denominator. If
  separately justified product work removes genuinely unreachable code, record
  the remaining platform or instrumentation limitations rather than exempting
  them from the measure.

## Constraints

- Tests use temporary directories and must not write into the repository's
  `context/` or `.runyte/`, nor into the person's configuration or platform cache
  directories. `external_open::cache_root` returns `None` under `cfg!(test)`,
  which does not protect integration tests under `tests/`; those must be given
  injected paths, as `tests/key_hints.rs` does.
- Never run a file a test wrote. Link to `src/fixtures/stand-in` and put the
  behavior beside the link in a `<program>.behavior` file.
- Tests that need a real external program, as `tests/lsp_real_servers.rs` does,
  must skip cleanly when it is absent rather than fail.
- Total suite runtime has to stay tolerable under instrumentation, which is
  slower than an ordinary run.
- `cargo fmt --check`, `cargo clippy --all-targets --locked -- -D warnings`, and
  `cargo test --locked` must pass.

## Recording

Each new baseline in `context/reference/test-coverage.md` records the tool
version, toolchain, target, covered and total counts, and the reason for changing
the floor. The README badge states the floor rather than a measured percentage,
so it changes in the same commit as the floor.
