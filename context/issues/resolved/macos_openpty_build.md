---
title: "Runyte 0.0.37 does not compile on macOS"
status: resolved
reported: 2026-08-20
resolved: 2026-08-20
legacy_commit: 18870df
---

## Resolution

Commit `18870df` (`Fix macOS PTY compilation`) corrected the pointers
`terminal::pty::open_pair` passes to `libc::openpty`. The function supplied a
const null termios pointer and a shared reference to the initial window size.
That compiled against glibc, whose declaration takes const pointers for those
two inputs, but not against Apple's libc declaration, which takes mutable raw
pointers.

The call now supplies a mutable null pointer and obtains a mutable raw pointer
to the window size with `addr_of_mut!`. Mutable raw pointers satisfy both libc
declarations: Apple and several BSDs accept them directly, while Rust coerces
them to the const pointers required on Linux. The termios pointer remains
null, and `openpty` continues to receive the same initialized window size, so
the change affects the Rust FFI types rather than PTY behavior.

Covered by `a_child_sees_the_size_the_pty_was_opened_with` in
`src/terminal/pty.rs`, which opens a real PTY and verifies that its child sees
the requested initial dimensions. `cargo fmt --check`,
`cargo clippy --all-targets -- -D warnings`, and the complete `cargo test`
suite pass.

## Report

Building Runyte v0.0.37 on macOS failed with this error:

```text
error[E0308]: arguments to this function are incorrect
    --> /Users/user/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/runyte-0.0.37/src/terminal/pty.rs:224:9
     |
 224 |         libc::openpty(
     |         ^^^^^^^^^^^^^
     |
note: types differ in mutability
    --> /Users/user/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/runyte-0.0.37/src/terminal/pty.rs:228:13
     |
 228 |             std::ptr::null(),
     |             ^^^^^^^^^^^^^^^^
     = note: expected raw pointer `*mut termios`
                found raw pointer `*const _`
note: types differ in mutability
    --> /Users/user/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/runyte-0.0.37/src/terminal/pty.rs:229:13
     |
 229 |             &size,
     |             ^^^^^
     = note: expected raw pointer `*mut winsize`
                  found reference `&winsize`
note: function defined here
    --> /Users/user/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/libc-0.2.189/src/unix/bsd/apple/mod.rs:4699:12
     |
4699 |     pub fn openpty(
     |            ^^^^^^^

For more information about this error, try `rustc --explain E0308`.
error: could not compile `runyte` (lib) due to 1 previous error
error: failed to compile `runyte v0.0.37`, intermediate artifacts can be found at `/var/folders/n2/lxbyt_z502qcnd9n_rdwsl580000gp/T/cargo-installtgi2dM`.
To reuse those artifacts with a future compilation, set the environment variable `CARGO_BUILD_BUILD_DIR` to that path.
```
