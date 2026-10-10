# Local compatibility patch

Source: crates.io `block` 0.1.6, by Steven Sheldon, declared MIT in its
preserved package manifest. The published archive and upstream repository
do not contain a separate license file.

GPUI 0.2.2 uses this crate directly and through Cocoa, Core Video, and Metal
on macOS. The private `Class` placeholder is now an inhabited opaque C struct
instead of an empty enum. The external `_NSConcreteStackBlock` static can
therefore legally exist; only its address is used, and block layout and
calling conventions remain unchanged. Every implicit `extern` ABI is made
explicitly `"C"`, preserving its meaning without the deprecated spelling.
No lint is suppressed.

The manifest's development-only `test_utils` path is removed because that
directory is absent from the published archive; its registry version is
retained. All other upstream source and metadata are preserved.

`tests/native/block_compat.rs` checks stack invocation, heap copying,
reference-counted cloning, and captured-value destruction on macOS CI.
Remove this patch when the native dependency graph no longer needs it.
