# Native compilation reports a future Rust incompatibility

`cargo build --release --features native` completes successfully but reports
that `proc-macro-error2 v2.0.1` contains code a future Rust version will reject.
Cargo suggests `--future-incompat-report` or
`cargo report future-incompatibilities --id 1` for details.

The build should complete without this dependency warning. The fix must retain
the optional native build and must not suppress the compiler diagnostic.
