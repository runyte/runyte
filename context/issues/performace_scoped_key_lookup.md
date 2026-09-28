# P3 — Scoped key lookup repeatedly rebuilds effective bindings

Priority: P3 (low). The measured overhead is avoidable on every scoped lookup,
but remains well below one millisecond in isolation.

At commit `22fd664`, `Keymap::lookup_in` calls `bindings_for_scope` in
`src/keymap.rs`. The latter allocates and sorts binding lists and, for a
non-global scope, searches the binding collection again for each global
binding to determine shadowing. Ordinary Markdown buffers use the Markdown
scope, so movement keys encounter this work even when their bindings match
the global defaults.

An exploratory measurement on 2026-09-28 used Linux x86-64, an AMD Ryzen AI 9 365,
and `cargo build --release --locked --lib`. After ten warmups, 1,000 samples
of default-keymap Normal-mode lookup for `l` gave medians of 3.2 microseconds
for `BindingScope::Global` and 74.3 microseconds for `BindingScope::Markdown`.
These measure one lookup, not the complete input path or a Helix comparison.

To reproduce, obtain `default_keymap()`, construct a `KeySequence` from
`KeyStroke::char('l')`, and repeatedly call `lookup_in(Mode::Normal, scope,
&sequence)` for those two scopes, consuming the result with `black_box`.
Use an optimized build and exclude keymap construction from the timed region.

Effective bindings should be prepared when a keymap or its runtime scopes
change, rather than reconstructed for every key. A candidate fix is a cached
lookup index per effective mode and scope. Preserve exact/prefix ambiguity,
binding role ordering, scoped shadowing, configured-keymap rollback, and
runtime plugin binding changes. Dispatch, help, and hints must continue to
read the same registry; consult `context/reference/helix-keymap-v1.md` before
implementation.

Validation should compare indexed results with current effective-scope
behavior for built-in, configured, and plugin bindings, including incomplete
sequences, and repeat the lookup measurements.
