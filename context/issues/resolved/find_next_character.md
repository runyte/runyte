---
title: "Character find is not discoverable from a prefix namespace"
status: resolved
reported: 2026-08-09
resolved: 2026-08-09
legacy_commit: 672a436
---

## Resolution

Commit 672a436 (`Make character find discoverable`) fixed the registry assembled
by `built_in_bindings`: `FindNextChar` was exposed only through the bare `f`
binding, so the prefix-driven key hints had no way to reveal it. The registry
now also maps `Space s f` to the same semantic command and advertises `f` as
its short alias. This keeps dispatch, help, and key hints on the shared keymap
boundary instead of adding a second input path.

The README editing table was audited against every one-key Normal/Select
binding and completed with the direct actions that were absent. It also makes
the scope of that inventory explicit. The Helix reference records the new
canonical path and the deliberate retention of `f` as the short spelling.
Character find continues to search across lines, matching the existing Runyte
and Helix behavior rather than limiting the new alias to the current line.

Tests covering this behavior are
`find_next_character_has_a_discoverable_space_binding_and_short_alias` in
`tests/keymap.rs`, `nested_space_tree_is_exact_primary_and_keeps_fast_compatibility_paths`
in `tests/keymap.rs`, and
`the_search_namespace_and_its_prompts_are_discoverable_on_screen` in
`tests/key_hints.rs`. The registry-size assertions in
`command_inventory_classifies_every_command_and_current_binding` in
`src/app.rs` cover the added binding in both modal modes.

## Report

The "find next character" action bound to `f` was undiscoverable, and it was
unclear how many other bare-key actions existed.

It should move under `Space s f`, with `f` retained as an alias. Every action
bound directly to a bare key, rather than under `Space`, `g`, or `Ctrl-w`,
should also be enumerated.
