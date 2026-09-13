---
title: "Plugin bindings under an undeclared prefix were listed flat in key hints"
status: resolved
reported: 2026-09-13
resolved: 2026-09-13
commit: a5502cc
---

## Resolution

Commit `a5502cc` (`Fold key hints under prefixes no namespace labels`) made
the key-hint popup group bindings under any prefix, not only under the ones
the built-in keymap declares.

`KeyHintState::rows_in` in `src/key_hints.rs` built namespace rows only from
`Keymap::namespaces_for_scope`, the hand-declared `BindingNamespace` list in
`src/keymap.rs`. A continuation deeper than the next key was hidden only when a
declared namespace covered it; otherwise it was listed as its own row with its
full sequence. Plugin bindings come from configuration at runtime and have no
declared namespace, so every binding under a shared prefix such as `Space =`
appeared directly in the `Space` popup.

`fold_undeclared_prefixes` now groups the continuations that no declared
namespace claims by their next key. A group becomes one namespace row,
`KeyHintRow::from_fold`, when it holds more than one binding and none of them
is bound to the prefix itself. A lone binding is left in full because folding
it would add a key press without making anything easier to find, and a prefix
that is also a command keeps its exact row and its continuations. The folded
row takes the most prominent role among its bindings and carries no
capability; each binding keeps its own availability when the row is entered.

`fold_label` names the row after the command name the bindings share: the
`::name` part of a plugin binding's description, or the command name of an
editor or colon target. `shared_command_stem` cuts that shared text at a `-`
or `.` separator or the end of a name, so `::time`, `::time-add`, and
`::time-pause` give `::time commands`, while `::task-add` and
`::tasking-list` share nothing. When there is no shared name, or the label
with its namespace marker would exceed the 44-cell description limit, the row
is labelled by count, such as `3 commands`.

The fold is computed from the effective keymap each time hints are drawn, so
it applies equally to plugin bindings, configured `keys`, and bindings a
plugin registers after startup. It does not add a `BindingNamespace`, so
keymap validation, dispatch, and help are unchanged. `docs/plugins.md`,
`docs/user-guide.md`, and `context/reference/helix-keymap-v1.md` describe
the behavior.

Regression coverage is in `src/key_hints.rs`:

- `plugin_bindings_under_an_undeclared_prefix_fold_into_one_namespace_row`
  binds five plugin commands under `Space =`, confirms `Space` shows one
  `::time commands ›` row beside the declared namespaces, and that `Space =`
  lists the five commands;
- `undeclared_prefixes_fold_only_groups_and_name_them_by_what_they_share`
  covers a dotted full command name, the count fallback, the separator rule,
  and a lone binding that stays unfolded;
- `built_in_keymaps_declare_every_namespace_their_hints_show` walks every
  prefix of every binding in the default and fast-pane keymaps and asserts
  that each namespace row there is a declared one, so built-in hints keep
  their hand-written labels.

Known limitation: `Keymap::entry_points` still labels a first key only from
declared namespaces, so a single-key prefix that only plugin bindings use has
no description in views that list entry points.

## Report

Plugin commands bound under a shared prefix were not grouped in the key-hint
popup. With the ru-time plugin configured with these bindings:

```yaml
"bindings": {
  "open": "Space = =",
  "add": "Space = a",
  "pause": "Space = p",
  ...
}
```

pressing `Space` listed every binding individually beside the built-in
namespace rows:

```
Space = =   ::time — Open time tracker
Space = a   ::time-add — Add a task
Space = d   ::time-delete
Space = n   ::time-note — Add or edit this task’s note
Space = p   ::time-pause — Pause the running timer
```

Expected behavior: the bindings fold into one `Space =` namespace row, the way
built-in groups such as `Space b` **Buffers** do, and pressing `=` reveals
them. Folding must happen automatically for every shared prefix, including
prefixes created dynamically by a plugin's bindings, without declaring them in
the built-in keymap.

The report did not specify the label of a folded row.
