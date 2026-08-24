---
title: "Special buffers shadow global key bindings"
status: resolved
reported: 2026-08-18
resolved: 2026-08-18
legacy_commit: 9b3ec31
---

## Resolution

Commit 9b3ec31 (`Move special-buffer actions behind Tab`) fixed the collision
between scoped buffer bindings and the global keymap. `built_in_bindings` in
`src/keymap.rs` had registered Git actions directly on ordinary modal letters,
and `bindings_for_scope` consequently hid the global binding whenever one of
those generated buffers was active. The scoped lookup machinery remains able
to diagnose such a collision, but the built-in registry now treats
`shadowed_bindings` as an invariant guard: every built-in scope must leave the
global map intact.

The keymap registry now owns `ContextAction` entries separately from direct
bindings. Each entry has a scope, a menu-local mnemonic, a typed command target,
a description, and a row-or-buffer classification. `App::open_context_actions`
reads those entries for the active scope, and the resulting overlay owns input
until it runs an action or is cancelled. Mnemonics run directly; arrows,
`j`/`k`, Shift-Tab, and Enter provide navigable access; Tab, Escape, and Ctrl-c
cancel. Registry construction rejects duplicate mnemonics and the reserved
menu controls. Help and the menu render the same registry entries, so their
action names and ordering cannot drift.

Tab no longer aliases jump-forward. `Ctrl-i` retains that command on Unix
terminals other than macOS, where enhanced keyboard reporting distinguishes it
from Tab. On macOS and Windows the terminal input cannot distinguish the two,
so jump-forward has no key; `Ctrl-o` remains jump-backward everywhere. Before
intercepting Tab, `App::handle_key_stroke` now also checks
`grammar.awaiting_character()`, ensuring commands such as `r`, `f`, `t`, `F`,
`T`, and `"` consume or reject Tab themselves and clear their pending state.

Special Git views retain only globally unbound direct keys such as Enter and
the Git log's Ctrl-n/Ctrl-p paging. Row-specific Git operations moved to the
Tab menu. Commit and refresh did not become menu entries because `Space g c`
and `Space g r` already provide the same buffer-wide operations. Pull and push
remain menu actions because they have no `Space g` spelling; pull is classified
as buffer-wide in both the changed-file and branch lists because it operates
only on the current checked-out branch, while push remains row-specific in the
branch list. Ordinary language-server buffers use Tab to request code actions,
and buffers with neither registry actions nor a language server report that no
actions are available.

The deliberate deviation from Helix is recorded in
`context/reference/helix-keymap-v1.md`. Per-view help no longer has a
`Different here` section; `Buffer keys` describes direct scoped keys and the
registry-backed Tab menu instead.

Coverage is provided by
`keymap::tests::no_scoped_binding_shadows_a_global_binding`,
`keymap::tests::contextual_actions_are_ordered_and_registry_backed`, and
`keymap::tests::contextual_action_mnemonics_cannot_take_menu_controls` in
`src/keymap.rs`; `app::tests::tab_does_not_bypass_a_command_waiting_for_a_character`,
`app::tests::tab_requests_code_actions_in_an_ordinary_language_buffer`,
`app::tests::n_creates_a_branch_at_the_selected_row_and_switches_to_it`, and
`app::tests::every_key_the_changed_file_list_advertises_does_what_it_says` in
`src/app.rs`; `help::tests::a_scoped_read_only_view_documents_what_only_it_does`
and `help::tests::an_action_only_scope_has_one_gap_before_its_tab_explanation`
in `src/help.rs`; and `worktree_removal_is_scoped_only_to_the_worktree_list` in
`tests/keymap.rs`.

Known limitation: on macOS and Windows, terminal input does not distinguish
Tab from Ctrl-i, so the forward jumplist command has no reachable key there.

## Report

Special buffers rebound keys that meant something else in every other buffer,
and the overridden key depended on the active buffer. The help for `Space g l`
made the collision visible as:

```
Different here
  These keys mean something else in every other buffer.

  n           Create a branch at the one on this line — elsewhere: Select only the next search match
  p           Fast-forward the current branch onto what it tracks — elsewhere: Paste after the selection
  P           Publish this branch to what it tracks — elsewhere: Paste before the selection
```

`p` and `P` were defensible on their own because a read-only buffer refused a
paste anyway. `n` was not: searching worked in a read-only buffer, so the
branch list took away the key used to browse matches. The complete collision
set was:

| Scope | Keys taken | What they meant elsewhere |
| --- | --- | --- |
| GitStatus | `o` `s` `u` `c` `p` `P` | open line below, **search**, undo, change, paste after, paste before |
| GitBranches | `n` `p` `P` | **search next**, paste after, paste before |
| GitWorktrees | `n` `N` `r` | **search next**, **search previous**, replace character |
| GitLog | `r` | replace character |
| GitStash | `a` `r` | append after, replace character |
| Diff | `s` `u` | **search**, undo |

The required rule was that a special buffer never shadow a global binding.

Several existing bindings already complied. Enter had no global Normal-mode
binding; it was bound only in Insert mode and within scopes, so it could remain
the primary action for the row under the cursor. Backspace, `-`, and `.` in the
explorer, `q` in help, and Ctrl-n/Ctrl-p in the Git log were also globally
unbound and did not need to move.

Every other contextual action was to move into a menu opened with Tab. Tab was
to become the action menu in every buffer, answering what could be done with
the object under the cursor. Existing pickers already followed that pattern:
Tab on a `:wls` or buffer-list row opened a menu through
`open_workspace_actions` or `open_buffer_actions` in `src/app.rs`. An ordinary
file buffer was to offer language-server code actions there or report that
none were available.

Tab was previously a second spelling of jump-forward and was to lose that job.
Ctrl-i was to retain jump-forward, while Ctrl-o remained jump-backward. The
comment near the binding in `src/keymap.rs` had said that terminals sent the
same byte for Tab and Ctrl-i and that Runyte did not enable a protocol to tell
them apart. That was out of date because `src/main.rs` enabled
`REPORT_ALL_KEYS_AS_ESCAPE_CODES` on Unix other than macOS. The accepted
platform behavior was that Unix terminals other than macOS could distinguish
the keys, while macOS and Windows could not and would therefore have no
jump-forward key after Tab became the menu. Ctrl-o would continue to walk the
jump list backwards on every platform.

Each menu entry was to carry a mnemonic active only while the menu was open:
`Tab s` stages, `Tab D` discards, and `Tab n` creates a branch. Arrow keys or
`j`/`k` plus Enter were to reach the same entries, and Escape was to cancel.
The menu was to list actions for the row under the cursor first, followed by
actions belonging to the buffer as a whole. In the changed-file list, the
requested row actions were stage, unstage, discard, and open, followed by the
buffer-wide actions. The earlier ordering example named commit as the
buffer-wide entry, while the later duplicate-command rule explicitly said
commit should not reappear because `Space g c` already reached it.

Commands that already had an equivalent global spelling were not to be copied
into the menu. `Space g s`, `Space g u`, `Space g c`, `Space g D`, and
`Space g r` already staged, unstaged, committed, discarded, and refreshed.
Scoped `s`, `u`, `c`, `D`, and `r` spellings that made the same request were to
be deleted. A menu entry remained appropriate where the row-scoped operation
was genuinely different from the global one, such as staging the file on the
selected row rather than the active file.

The keymap registry was to remain the single source of truth. `src/help.rs` and
`src/key_hints.rs` already read scoped bindings from it, and the action menu was
to read the same registry rather than keep its own action list. Once no global
keys were shadowed, the per-view help's `Different here` section was to be
removed and `Buffer keys` was to explain what Tab opened.

`context/reference/helix-keymap-v1.md` recorded Tab as jump-forward and needed
to record the deliberate divergence.
