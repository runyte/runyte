# Test coverage

This register records Runyte's source-based Rust coverage baseline and the CI
floor derived from it. It measures which instrumented code the ordinary test
suite executes; it does not replace review of test quality, platform behavior,
or uninstrumented external programs.

The canonical command is:

```sh
cargo llvm-cov --locked --workspace
```

## Target measure

The above-95% target applies to the total **Lines** percentage printed by that
canonical command. It is the only current measure that `cargo-llvm-cov` can
enforce directly and identically in a local run and in CI. It is a reported
source-coverage figure, not a claim that more than 95% of production-only Rust
source has run: stable Rust still instruments inline `#[cfg(test)]` modules in
the same source files as production code, and `cargo-llvm-cov` cannot exclude
only those portions of a file.

A custom source parser that subtracts test item ranges would introduce a
second, Rust-syntax- and configuration-sensitive coverage implementation that
the compiler does not verify. Runyte therefore does not use such a figure as a
gate. New behavior coverage should preferentially live in standalone files
under `tests/` or existing source subdirectories named `tests`, which
`cargo-llvm-cov` excludes, so adding a test does not itself make the target
easier. Revisit this decision if stable Rust gains a compiler-owned way to
exclude inline test code from source coverage.

The 95% floor may be enabled only after the canonical command clears it on both
`x86_64-unknown-linux-gnu` and `aarch64-apple-darwin`. Until then, CI keeps a
lower floor that holds on its measured target, and each platform baseline is
recorded separately.

CI uses `cargo-llvm-cov` 0.9.0, publishes the full per-file summary in the job
summary, retains an HTML report as the `rust-coverage-html` artifact for 14
days, and fails below 86% total line coverage. The floor is deliberately below
the observed baseline because conditional Linux and macOS code changes both the
instrumented denominator and the paths available to a run on one platform.

## 2026-09-02 — macOS

Measured with `cargo-llvm-cov` 0.9.0 and Rust 1.97.1 on
`aarch64-apple-darwin` in GitHub Actions run 153. The job verified the host
target before measuring, and the ordinary non-ignored workspace tests passed
under instrumentation.

| Measure | Covered | Total | Coverage |
| --- | ---: | ---: | ---: |
| Lines | 90,481 | 100,380 | 90.14% |
| Functions | 8,450 | 9,253 | 91.32% |
| Regions | 140,408 | 156,605 | 89.66% |

This is the first macOS measurement after the behavior-focused coverage pass.
It supersedes the 2026-08-30 macOS baseline for current floor decisions and
confirms that total line coverage exceeds 90% on both first-class targets. The
enforced floor is raised from 83% to 86%: this turns a material regression red
while retaining 4.14 percentage points of headroom below the lower measured
platform. The above-95% target has not been reached, and 90.14% leaves too
little platform headroom to make 90% a useful regression gate.

## 2026-09-02 — Linux

Measured with `cargo-llvm-cov` 0.9.0 and Rust 1.97.1 on
`x86_64-unknown-linux-gnu`. The ordinary non-ignored workspace tests passed
under instrumentation.

| Measure | Covered | Total | Coverage |
| --- | ---: | ---: | ---: |
| Lines | 90,593 | 100,194 | 90.42% |
| Functions | 8,449 | 9,238 | 91.46% |
| Regions | 140,633 | 156,351 | 89.95% |

The fresh same-tree baseline before the continuation pass was 90,308 of
100,134 lines (90.19%), 8,438 of 9,234 functions (91.38%), and 140,184 of
156,240 regions (89.72%). The earlier recorded line baseline covered seven
fewer lines because concurrent paths varied between instrumented runs; the
before-and-after comparison here uses the fresh run. Covered lines increased
by 285 while the instrumented total increased by 60, leaving 225 fewer
uncovered lines and raising total line coverage by 0.23 percentage points.

The added tests cover Git cancellation outcomes, unborn-branch pull and rebase
refusals, untracked-branch push remote selection, language-server launch and
JSON-RPC failures, diagnostic clearing, notification filtering, malformed
workspace-edit refusal, repeated local-protocol handshakes, partial and final
wait completion, and standalone path-completion and command-path rendering.
Review replaced fixed-delay LSP assertions with event or wire-message barriers,
made the failed-launch test incapable of executing a stale temporary file,
strengthened file-versus-directory UI assertions, and removed a Git outcome
matrix that enumerated variants without establishing distinct behavior.

The preceding behavior-focused pass had raised its own fresh same-tree Linux
baseline from 89,267 of 99,906 lines (89.35%) to the recorded 90,301 of
100,134 lines (90.18%), a gain of 0.83 percentage points. Its tests exercised
observable refusal, recovery, lifecycle, protocol, picker, Git, LSP, terminal,
and persistent-workspace behavior; coverage-only command and provider sweeps
were removed during review rather than retained for their reported gain.

The enforced floor is 86%. The 95% target has not been reached. The
latest macOS measurement above reached 90.14%, leaving too little headroom for
a 90% cross-platform floor. No post-change macOS measurement exists yet, so the
lower measured platform still provides 4.14 percentage points of headroom
above the floor; neither the floor nor the README badge changes.

## 2026-09-01

Measured with `cargo-llvm-cov` 0.9.0 and Rust 1.97.1 on
`x86_64-unknown-linux-gnu`. The ordinary non-ignored workspace tests passed
under instrumentation.

| Measure | Covered | Total | Coverage |
| --- | ---: | ---: | ---: |
| Lines | 85,649 | 96,014 | 89.20% |
| Functions | 8,085 | 8,902 | 90.82% |
| Regions | 133,747 | 150,380 | 88.94% |

Against a same-toolchain Linux run immediately before the added tests, the
line denominator stayed at 96,014 while covered lines rose by 385, from 85,264
to 85,649 (88.80% to 89.20%). Covered regions rose by 603. The largest direct
line gains were in `app/input.rs` (+163), `protocol/input.rs` (+69, reaching
100%), and `app/git_workflows.rs` (+58); behavior reached through those
boundaries also covered lines in prompt editing, the finder, file picker, host,
transport, and event-loop coordination.

The enforced floor remains 83%. The target has not yet been reached, and the
macOS baseline below predates these tests; a current macOS run is required
before any cross-platform floor increase.

## 2026-08-30

Measured with `cargo-llvm-cov` 0.9.0 and Rust 1.97.1 on
`aarch64-apple-darwin`. The ordinary non-ignored workspace tests passed under
instrumentation.

| Measure | Covered | Total | Coverage |
| --- | ---: | ---: | ---: |
| Lines | 78,625 | 93,004 | 84.54% |
| Functions | 7,467 | 8,604 | 86.79% |
| Regions | 123,133 | 145,814 | 84.45% |

The enforced line floor begins at 83%. A later baseline should record the tool,
toolchain, target, covered and total counts, and the reason for changing the
floor.

The README coverage badge states this floor rather than a measured
percentage, so changing the floor means editing the badge in the same
commit.

### Interpretation

`cargo-llvm-cov` excludes standalone files under directories named `tests`, but
stable Rust cannot yet mark every inline `#[cfg(test)]` module as excluded from
coverage. Some inline test code is therefore part of both the instrumented
denominator and the covered count. This baseline is useful for regression
detection within the same setup, but its percentage should not be read as the
share of production-only source that tests execute.
