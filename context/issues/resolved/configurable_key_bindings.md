---
title: "Key bindings cannot be changed in the configuration"
status: resolved
reported: 2026-09-03
resolved: 2026-09-04
commit: f4ab8f4
---

## Resolution

Commit `f4ab8f4` (`Add configurable key remapping`) made the effective keymap
an editor-owned value assembled from the built-in registry and the active
configuration. Previously, `keymap_for` could only select one of two static
maps, `App` borrowed that map for the process lifetime, and `KeyStroke` had no
text parser. Consequently, configuration could neither identify bindings nor
produce the same registry consumed by dispatch, help, and key hints.

`src/keymap/configured.rs` now compiles `keys.leader`, `keys.window`, and
`keys.rebind` into both normal and fast-pane variants. Rebind left-hand sides
name default spellings; right-hand sides are absolute and may use the
configured `Leader` and `Window` tokens. Prefix rewrites use the longest
matching default prefix, advertised aliases participate in the same registry,
and the compiler applies bounded admission, conflict validation, provenance
tracking, and per-entry rollback. Invalid entries retain the affected built-in
bindings and produce a non-fatal `Key bindings` startup notification instead
of preventing the editor from opening; malformed YAML remains a hard load
error. `src/keymap/validate.rs` applies the same structural checks to built-in
and configured maps, while `KeyStroke::parse` is the inverse of canonical key
labels.

`App` now owns its keymap through `Arc`, rebuilding it for configuration
previews, saves, rollbacks, and `editor.fast_pane_keys` changes. Dispatch,
key-hint rows, generated help tables, the manual, About page, tutorial, action
messages, overlay dismissal, and the integrated terminal's reserved window
prefix all read the effective registry or its metadata. `src/key_spelling.rs`
provides explicit binding, prefix, key, and literal-key substitutions so prose
can distinguish keys that move from literal input. The default-key register
and user guide document the syntax, scope, failure behavior, and intentional
differences from Helix.

The parser round trip is covered by
`canonical_key_labels_parse_back_to_the_same_binding_identity` in
`src/input.rs`. Compilation and rollback are covered in
`src/keymap/configured.rs` by
`longest_rebind_wins_and_named_prefixes_expand`,
`leader_and_literal_space_have_distinct_meanings`,
`advertised_alias_moves_with_its_registry_advertisement`,
`invalid_window_and_conflicting_rules_are_rolled_back`, and
`rules_that_empty_a_namespace_are_attributed_and_rolled_back`, together with
the compiler-bound and variant-count tests. Shared invariants are covered by
`every_structural_violation_class_is_detected` and
`both_built_in_variants_satisfy_the_shared_validator` in
`src/keymap/validate.rs`. Live spelling across both variants is covered by
`sentinel_map_moves_both_prefixes_a_namespace_and_an_alias_in_both_variants`
and the inventory tests in `src/key_spelling.rs`. Application tests in
`src/app/tests/mod.rs` cover configured leaders, malformed and null settings,
variant-specific behavior, and live surfaces; picker dismissal is covered in
`src/app/tests/search_and_pickers.rs`; terminal prefix reservation and
canonical shifted input are covered by
`configured_window_prefix_is_reserved_and_control_w_returns_to_the_child` and
the related tests in `tests/terminal.rs`.

Known limitation: remapping is intentionally bounded rather than a general
keymap language. It cannot unbind commands or introduce a binding for a
command without an eligible default, and rebind left-hand sides are limited to
the default `Space` and `Ctrl-w` namespaces plus advertised aliases. Because a
left-hand side names a default spelling, a later release that changes that
default reports the rule as unmatched. Typed colon-command names do not
change, although their existing key aliases may move. The configured window
prefix remains reserved by the editor while a terminal has focus.

## Report

Every binding was compiled in. A reader who wanted `Space g c` instead of
`Space g l`, or who could not press Space comfortably, had no configuration
for doing so.

`src/keymap.rs` was the single declarative source of truth:
`built_in_bindings()` returned a static list, `build_keymap` produced two
`LazyLock` statics, and `keymap_for(fast_pane_keys)` returned one of them as an
`&'static Keymap`. `App` held `keymap: &'static Keymap`. `Keymap::new` rejected
duplicate sequences with `DuplicateBinding`, and the built-in uniqueness
invariant was covered by `every_mode_sequence_is_unique_and_described` in
`tests/keymap.rs`, but there was no path for user configuration to participate
in that validation.

There was no leader abstraction. Every application sequence used
`Key::char(' ')` literally, and `Ctrl-w` likewise headed the window
sequences. `src/config.rs` loaded `config.yaml`; malformed YAML and settings
rejected by `validate_settings` aborted startup. The existing `themes` map was
the precedent for a user-defined map. `KeyStroke::label()` printed keys and
`KeyStroke::canonical_for_binding` normalized them, but no text parser existed.

The requested behavior was:

- `config.yaml` can rebind sequences in the `Space` and `Ctrl-w` namespaces.
- A whole prefix can move, so `Space g` to `Space G` carries every descendant
  and the namespace label.
- Space is the leader key, and it can be replaced by another key or a modified
  key such as `Ctrl-x`.
- Typed colon-command names remain stable and cannot themselves be rebound.
- Dispatch, key hints, help, and the alias column of hint rows all reflect the
  effective binding; no surface describes a displaced default.
- One sequence cannot reach two commands. Conflicts are detected and reported
  at startup without leaving the editor unusable.
- The syntax remains readable.

The selected configuration shape expresses rewrites from default spellings on
the left to effective spellings on the right:

```yaml
keys:
  leader: Ctrl-x
  window: Ctrl-a
  rebind:
    Space g: Leader G
    Space g l: Leader G c
    Ctrl-w x: Window e
    Space e: Space
    ",": F12
```

The left-hand side is interpreted against the release's default map, while the
right-hand side is absolute. `Leader` and `Window` expand to their configured
keys; literal `Space` and `Ctrl-w` remain available as ordinary keys. The
longest matching prefix wins. An unmatched left-hand side is reported, which
prevents a changed default in a later release from being ignored silently.
This shape supports a single binding, a whole prefix, and leader/window moves
without requiring a separate prefix configuration type. A command-keyed map
such as the following was considered but would need a second mechanism for
prefix moves:

```yaml
keys:
  leader: Ctrl-x
  bindings:
    git-log: Space g c
```

Key text is required to use the spellings the editor prints, including
`Space`, `Ctrl-w`, `Alt-j`, `Up`, and `Tab`, with parsing inverse to
`KeyStroke::label()` after binding canonicalization. Reported failure classes
include an unparsable key, an unmatched default sequence, two entries reaching
the same effective sequence, an entry colliding with a binding outside the
configurable namespaces (for example, choosing `s` as leader would shadow
search), and a rewrite that empties a required namespace. The notification
names the offending entry and retains the applicable built-in behavior.

The effective keymap must be owned by the editor and rebuilt wherever
configuration moves: initial load, setting preview, save, rollback, and
`editor.fast_pane_keys` changes. Short aliases advertised by configurable
namespaces, including `,` for `Space s c`, `&` for `Space s a`, and the
finder's short spelling, move with their registry advertisements. The
restricted window-command set available inside a terminal follows the
configured window prefix.

Bare Space in modal result lists, choice overlays, action menus,
confirmations, and the initial picker cancellation had also served as a
symmetric dismissal key. Those semantic leader dismissals follow a changed
leader; text-entry surfaces that describe or accept literal Space retain it.

Dispatch, help, and key hints must continue to read one registry.
`context/reference/helix-keymap-v1.md` describes the built-in defaults, while
`docs/user-guide.md` documents the configuration beside settings and themes.
Runtime state remains outside `context/`; this feature reads the person's
`config.yaml` and never writes changes back to it.

Regression coverage must exercise the resulting keymap rather than only the
file format: a rebound sequence dispatches and its old spelling stops doing
so; prefix moves carry descendants and namespace labels; leader changes reach
every application binding; all conflict classes identify the offending entry
and preserve a usable editor; and key hints and help show the configured
spellings.
