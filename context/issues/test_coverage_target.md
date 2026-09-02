# Raise measured test coverage above 95%

Linux and macOS are Runyte's first-class platforms, and the test suite is the
main evidence that both stay correct as the editor changes.

`context/reference/test-coverage.md` records the current baseline, measured with
`cargo-llvm-cov` 0.9.0 and Rust 1.97.1 on `aarch64-apple-darwin`: 78,625 of
93,004 lines, 7,467 of 8,604 functions, and 123,133 of 145,814 regions. CI
publishes the per-file summary, retains an HTML report, and fails below an
enforced 83% line floor.

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

The issue remains open. The current Linux result has 9,833 uncovered lines.
The last recorded macOS baseline predates this pass, so a current macOS
measurement is also still required before raising the cross-platform floor.
The CI floor and README badge remain at 83% until the target holds on both
first-class targets.

## Pending macOS baseline

The standard GitHub-hosted `macos-latest` runner is currently an Apple Silicon
runner, so CI can produce the required `aarch64-apple-darwin` baseline without
a larger or self-hosted runner. The existing macOS job only runs the ordinary
test suite and does not collect coverage.

A future pull request may add a temporary, non-gating `coverage-macos` job that
uses `macos-latest`, sets `TMPDIR=/private/tmp`, and follows the pinned Linux
coverage recipe. The job must verify the target before measuring so a change to
GitHub's moving runner alias cannot silently record an Intel baseline:

```sh
test "$(rustc -vV | sed -n 's/^host: //p')" = aarch64-apple-darwin
cargo llvm-cov --locked --workspace --no-report
cargo llvm-cov report | tee coverage-summary-macos.txt
```

The job should install `cargo-llvm-cov` 0.9.0, use a distinct macOS coverage
cache key, publish the summary in the job summary, and retain it under a unique
artifact name such as `rust-coverage-macos-arm64`. It must not enforce a new
floor on its first run. Record the resulting toolchain, target, covered and
total counts in `context/reference/test-coverage.md` before deciding whether
the macOS coverage job should remain in CI or whether the cross-platform floor
can change.

## What the number has to mean first

The recorded baseline counts some inline `#[cfg(test)]` modules in both the
covered count and the total, because stable Rust cannot mark every one of them
as excluded. The reported percentage therefore moves when the ratio of inline
test code to production code moves, without any behavior becoming better tested.
Standalone files under `tests/` are already excluded.

Before the target is pursued, decide and record whether it applies to the
reported figure or to a production-only figure, and if the latter, how that
figure is produced and whether it can be enforced in CI. A 95% floor on a
measure that inline test code inflates is weaker than the number suggests.

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
- Where code is genuinely unreachable, reduce it rather than exempt it where
  that is possible, and record the remainder as a known share of the denominator.

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
- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, and
  `cargo test` must pass.

## Recording

Each new baseline in `context/reference/test-coverage.md` records the tool
version, toolchain, target, covered and total counts, and the reason for changing
the floor. The README badge states the floor rather than a measured percentage,
so it changes in the same commit as the floor.
