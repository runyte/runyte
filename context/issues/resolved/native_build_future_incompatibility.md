---
title: "Native compilation reports a future Rust incompatibility"
status: resolved
reported: 2026-10-08
resolved: 2026-10-08
commit: 2766482
---

## Resolution

Commit `2766482` — Fix native window font sizing, clipboard integration, and build warning.

The native dependency graph brought `proc-macro-error2` 2.0.1 through
GPUI 0.2.2 and stacksafe 0.1.4. Its `__export` module publicly re-exported
`proc_macro`, but the crate's `extern crate proc_macro` declaration was private.
Rust reported future-incompatibility E0365 even though Runyte compiled.

The checkout now patches that dependency with its source and licenses under
`vendor/proc-macro-error2`. The only Rust source change makes the existing
`extern crate` declaration public, as the compiler recommends. No lint is
suppressed and the GPUI version is unchanged. `RUNYTE-PATCH.md` records the
provenance and Apache-license line-ending normalization.

Validation: `cargo build --release --features native --future-incompat-report`
reported zero dependencies with future-incompatible warnings. Default and
native all-target Clippy passed. The `native_frontend` tests rooted in
`src/native_frontend.rs` passed (32 tests; the existing manual Poppler test
remains ignored in that invocation).

Known limitation: the compatibility patch is local to this checkout and should
be removed once the native dependency graph carries the upstream correction.

## Report

`cargo build --release --features native` completes successfully but reports
that `proc-macro-error2 v2.0.1` contains code a future Rust version will reject.
Cargo suggests `--future-incompat-report` or
`cargo report future-incompatibilities --id 1` for details.

The build should complete without this dependency warning. The fix must retain
the optional native build and must not suppress the compiler diagnostic.
