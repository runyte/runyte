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
days, and fails below 83% total line coverage. The floor is deliberately below
the observed baseline because conditional Linux and macOS code changes both the
instrumented denominator and the paths available to a run on one platform.

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
