---
title: "Scoped key lookup repeatedly rebuilds effective bindings"
status: resolved
reported: 2026-09-28
resolved: 2026-09-28
commit: bc38786
---

## Resolution

Commit `bc38786` (`Precompute effective scoped key lookups`) changed
`Keymap::lookup_in` to read a prepared index instead of calling
`bindings_for_scope` on every key. The old path allocated and sorted effective
bindings, then searched the registry again to decide whether each global
binding was shadowed by a scoped binding. That work also happened for a global
movement key in a Markdown buffer.

`Keymap::rebuild_lookup_index` prepares exact matches and ordered
continuations for each effective mode and scope. It stores positions into the
binding registry, so lookup returns the same bindings that dispatch, help,
and hints use. Scoped entries precede global entries, stable role ordering is
retained within each group, and a scoped sequence hides the matching global
sequence. Modes and scopes without scoped bindings use the global index. The
index is built with a new keymap and rebuilt after successful runtime plugin
binding replacement. Configured maps build their own index; rejected rules
roll back to an intact map. There is no change to Runyte's binding grammar or
its documented differences from Helix.

The repeatable release benchmark in `examples/keymap_lookup.rs` measured
Normal-mode `l` lookups after warmup at 28.1 ns per lookup in both Global and
Markdown scopes. It uses 1,000 samples of 1,000 lookups each and excludes
keymap construction. The original issue's single-lookup samples used a
different timing method, so the numbers are not a controlled before/after
ratio. Neither measurement covers the complete input path.

Coverage is provided by
`keymap::tests::indexed_lookup_matches_effective_bindings_across_registry_changes`
and
`keymap::tests::indexed_lookup_preserves_ambiguity_role_order_and_scoped_shadowing`
in `src/keymap.rs`. They compare the index with the original effective-scope
lookup logic across built-in maps, valid and rejected configuration, runtime
plugin binding replacement, incomplete sequences, exact/prefix ambiguity,
role ordering, and scoped shadowing.

Known limitation: initial index construction cost and retained index memory
were not independently measured; the lookup benchmark cannot establish full
input latency.

## Report

Priority: P3 (low). The measured overhead was avoidable on every scoped
lookup, but remained well below one millisecond in isolation.

At commit `22fd664`, `Keymap::lookup_in` called `bindings_for_scope` in
`src/keymap.rs`. The latter allocated and sorted binding lists and, for a
non-global scope, searched the binding collection again for each global
binding to determine shadowing. Ordinary Markdown buffers used the Markdown
scope, so movement keys encountered this work even when their bindings
matched the global defaults.

An exploratory measurement on 2026-09-28 used Linux x86-64, an AMD Ryzen AI
9 365, and `cargo build --release --locked --lib`. After ten warmups, 1,000
samples of default-keymap Normal-mode lookup for `l` gave medians of 3.2
microseconds for `BindingScope::Global` and 74.3 microseconds for
`BindingScope::Markdown`. These measured one lookup, not the complete input
path or a Helix comparison.

To reproduce, obtain `default_keymap()`, construct a `KeySequence` from
`KeyStroke::char('l')`, and repeatedly call `lookup_in(Mode::Normal, scope,
&sequence)` for those two scopes, consuming the result with `black_box`. Use
an optimized build and exclude keymap construction from the timed region.

The expected behavior was for effective bindings to be prepared when a
keymap or its runtime scopes changed rather than reconstructed for every key.
A cached lookup index per effective mode and scope was a candidate approach.
Exact/prefix ambiguity, binding role ordering, scoped shadowing,
configured-keymap rollback, and runtime plugin binding changes needed to be
preserved. Dispatch, help, and hints needed to continue reading the same
registry, with `context/reference/helix-keymap-v1.md` consulted for binding
semantics.

Validation needed to compare indexed results with effective-scope behavior
for built-in, configured, and plugin bindings, including incomplete
sequences, and to repeat the lookup measurements.
