# Local compatibility patch

Source: crates.io `proc-macro-error2` 2.0.1 (MIT OR Apache-2.0).
The source files and license text are copied unchanged (the Apache license
uses LF line endings here), except that `src/lib.rs`
uses `pub extern crate proc_macro` so the existing public `__export` re-export
is legal. This fixes Rust future-incompatibility lint E0365 without suppressing
it. GPUI 0.2.2 brings this dependency through stacksafe 0.1.4.

Remove the patch when the native dependency graph no longer needs it.
