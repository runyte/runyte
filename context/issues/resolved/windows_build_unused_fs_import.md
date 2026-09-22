---
title: "Windows release build reports an unused fs import"
status: resolved
reported: 2026-09-21
resolved: 2026-09-21
commit: 7757bb7
---

## Resolution

Commit `7757bb7` (`Gate filesystem import by Windows build configuration`)
fixed the unconditional `std::fs` import in `src/main.rs`. Its production uses
on Windows are behind `debug_assertions`; the other uses are on non-Windows
platforms or in tests. A Windows release build therefore imported `fs` without
using it. The import now follows those compile conditions, including `test` so
release-profile tests can still use it.

Verification for `src/main.rs`: `cargo check --release --locked --bin runyte`
reproduced the warning before the change and passed without warnings afterward;
`cargo check --locked --bin runyte` and `cargo fmt --check` also passed.

## Report

On Windows, `cargo build` was reported to emit this warning:

```text
warning: unused import: `fs`
  --> src\main.rs:19:5
   |
19 |     fs,
   |     ^^
   |
   = note: `#[warn(unused_imports)]` (part of `#[warn(unused)]`) on by default
```

The expected behavior is a build without the unused-import warning. A local
`cargo check --release --locked --bin runyte` reproduced it on Windows; the
default debug profile did not reproduce it in the current source.
