# Test coverage

This register records Runyte's source-based Rust coverage baseline and the CI
floor derived from it. It measures which instrumented code the ordinary test
suite executes; it does not replace review of test quality, platform behavior,
or uninstrumented external programs.

The canonical command is:

```sh
cargo llvm-cov --locked --workspace
```

CI uses `cargo-llvm-cov` 0.9.0, publishes the full per-file summary in the job
summary, retains an HTML report as the `rust-coverage-html` artifact for 14
days, and fails below 83% total line coverage. The floor is deliberately below
the observed baseline because conditional Linux and macOS code changes both the
instrumented denominator and the paths available to a run on one platform.

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
