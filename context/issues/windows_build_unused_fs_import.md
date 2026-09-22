# Windows `cargo build` reports an unused `fs` import

On Windows, `cargo build` emits this warning:

```text
warning: unused import: `fs`
  --> src\main.rs:19:5
   |
19 |     fs,
   |     ^^
   |
   = note: `#[warn(unused_imports)]` (part of `#[warn(unused)]`) on by default
```

The build should complete without an unused-import warning. Reproduce by
running `cargo build` on Windows. The `fs` import is unconditional. In the
current source its uses include Unix-only code and a debug-only input trace;
the exact build configuration that produces the warning still needs to be
confirmed.
