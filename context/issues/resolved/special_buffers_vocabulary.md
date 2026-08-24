---
title: "Runyte-generated buffers have no shared vocabulary term"
status: resolved
reported: 2026-08-18
resolved: 2026-08-18
legacy_commit: 239ddb6
---

## Resolution

Commit 239ddb6 (`Define the special buffer vocabulary`) added **special
buffer** to `context/reference/ui-vocabulary.md` and introduced the term in
`README.md`. The definition is based on provenance: Runyte assembles the
buffer's contents instead of reading ordinary file text. It explicitly does
not use read-only state or the presence of a path as proxies, so the editable
explorer and commit-message buffer belong while a pathless scratch buffer does
not.

`BindingScope` in `src/keymap.rs` now documents every non-global variant as a
special-buffer scope. `BindingScope::is_special_buffer_scope` attaches the
term to the registry boundary used by the binding invariant, while noting that
the notification buffer and about page remain globally scoped because they
have no actions of their own. The vocabulary enumerates all twelve scoped
roles plus those two exceptions and states that a special buffer remains an
ordinary Runyte buffer rather than a menu or overlay.

Coverage is provided by
`keymap::tests::every_non_global_binding_scope_belongs_to_a_special_buffer` in
`src/keymap.rs`, whose fixed inventory count requires the canonical vocabulary
to be revisited when a scope is added.

## Report

`context/reference/ui-vocabulary.md` named **Buffer** and **Pane-backed
filterable list**, but did not name the category users encounter as buffers
Runyte produces itself rather than reading from files. The config view,
explorer, `Space g l`, `Space g b`, `Space g g`, notification buffer, help, and
about page were all instances of that unnamed category.

The requested name was **special buffer**.

The defining property was that Runyte generated the contents rather than
reading them from a file. Read-only state was not the test: the explorer and
commit-message buffer were generated and editable, and both belonged. Having
no path was not the test either: the scratch buffer had no path but remained
ordinary text with no actions of its own.

The scoped set was every non-global `BindingScope` variant in `src/keymap.rs`:
`Directory`, `Settings`, `GitStatus`, `GitBranches`, `GitWorktrees`, `GitLog`,
`GitBlame`, `GitStash`, `WorkspaceSearch`, `Help`, `CommitMessage`, and `Diff`.
The notification buffer and about page also belonged because Runyte generated
them the same way, although they did not yet have keys of their own. Keeping
the vocabulary category aligned with the scope enum gave the special-buffer
binding rule a named category to attach to.

A special buffer was not a menu and was not to be described as one. Reusable
results belong in buffers while choose-one requests belong in overlays. A
special buffer remains on the buffer side of that boundary and uses ordinary
Runyte motion, selection, search, splits, and jump history. What distinguishes
it is the origin of its text and its own contextual actions, not its rendering.
