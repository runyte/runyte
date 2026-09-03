---
title: "The search sigil and the finder had settled on spellings the registry did not hold"
status: resolved
reported: 2026-09-03
resolved: 2026-09-03
commit: 845cde9
---

## Resolution

Fixed in `845cde9`, "Give the search sigil and the finder one spelling each".

`built_in_bindings` in `src/keymap.rs` registered `/` as the alias of
`Space / /` for `open-file-picker` and gave the in-buffer regular-expression
search the free `S` key. That made `/` mean the finder at the top level and
search under the `Space /` prefix, and it left the finder and the search
competing for the sigil in the Git namespace, where `Space g /` had taken `/`
away from `f` on the stated grounds that `/` means search anywhere.

The registry now spells each idea once. `modal(Key::char('/'), …)` is
`search-regex`, `Space / /` is `global-search-regex`, and the pair mirrors
`s` and `Space / s`: the prefix widens a flavour rather than respelling it.
The finder takes `f` in every namespace — `Space / f` as the Primary binding
carrying `Space f` as its advertised alias, `Space f` registered separately so
dispatch reaches it, and `Space g f` for `:git-search-commits`. `Space f` picks
up `BindingRole::Fast` from `existing_binding_role`, which already listed `f`
among the short `Space` suffixes, so no role had to be stated at the call site.
`S`, `Space / S`, `Space g /`, and bare `/` as the finder are unbound.

Nothing above the registry needed a dispatch change: `search-regex` was already
reached by command identity in `src/app/input.rs` and
`src/app/terminal_workflows.rs`, so terminal review search follows the key
without knowing which one it is.

The three finder commands in `src/command.rs` now describe themselves with one
word — "Open the finder over the project's files, buffers, and terminals",
"… over the project, including files Git ignores", and "… in a chosen path,
including files Git ignores" — and the overlay title built in
`src/app/presentation.rs` and drawn in `src/ui.rs` reads `Finder · …` rather
than `Find · …`. `open-fuzzy-grep` is the fourth entry point to the same
surface and takes the same word, so the palette cannot name it a finder in
three places and something else in the fourth. The
`editor.show_hidden_files` description in `src/settings.rs` says finder rather
than file picker. Colon spellings (`:file-picker`, `:file-picker-all`,
`:file-picker-path`) are command identities and were left alone.

`context/reference/helix-keymap-v1.md` moved with the keys in the same commit.
Two passages needed argument rather than substitution. The paragraph justifying
`Space g /` reasoned that `/` means search in any namespace, which this change
inverts, so it was rewritten around `f` meaning the finder in any namespace
instead of left standing. The single-letter binding audit listed `S` among the
deviations that take a standard letter and change its meaning; it now records
`S` as unbound and dates the audit to this change, because an unbound letter
and a redefined one are different claims about the surface. `README.md`,
`docs/user-guide.md`, `context/reference/ui-vocabulary.md`, `src/manual.rs`,
and the `Space g /` mention in the open `search_overlay_query_line.md` follow
the registry.

Help prose in `src/help.rs` already described `s` and `/` as the two search
keys in both the text-buffer and terminal overviews. That prose was wrong while
`/` was the finder and is correct now, so it was left as written.

A follow-up commit finished two surfaces the first pass missed, both of which
still told a reader to reach for the retired `/`: the `:help` getting-started
topic in `src/manual.rs`, which is the first prose the general manual shows,
and the `src/finder.rs` entry in `AGENTS.md`, which is what the next agent
reads before touching this code. Neither is reachable from the registry, which
is why the keymap's own consistency tests could not catch them.

Tests, all in `tests/keymap.rs` unless noted:

- `the_finders_short_spelling_matches_its_namespace_spelling_in_every_scope`
  is new. It pins `Space f` and `Space / f` to one command identity in both
  modal modes, with the Fast/Primary roles and the advertised alias. It
  iterates `BindingScope::ALL` rather than a hand-listed subset, so a scope
  added later cannot shadow the finder unnoticed.
- `the_search_sigil_means_the_same_flavour_in_the_buffer_and_the_workspace` is
  new. It pins `s`/`/` and `Space / s`/`Space / /` to the four search
  identities, so the sigil cannot come to mean two things again.
- `finder_and_workspace_search_are_global_in_every_buffer_scope`,
  `nested_space_tree_is_exact_primary_and_keeps_fast_compatibility_paths`,
  `character_find_and_project_find_keep_distinct_direct_bindings`,
  `git_namespace_keeps_only_navigation_and_refresh_commands`, and
  `every_mode_sequence_is_unique_and_described` were updated to the new
  spellings rather than relaxed. The Fast set now holds `Space f` beside
  `Space e` and `Space E`.
- `removed_duplicate_bindings_stay_unbound` gained `S`, `Space / S`, and
  `Space g /`. Bare `/` reaching the finder is covered by the two tests above,
  which assert what it reaches instead.
- `the_search_namespaces_and_their_prompts_are_discoverable_on_screen` and
  `repeated_arrow_scroll_saturates_at_the_rendered_end` in `tests/key_hints.rs`
  pin the rendered `Space / f, Space f` alias row and the widened `Space`
  namespace.
- `user_guide_covers_every_direct_editing_binding` in `src/keymap.rs` now
  requires the guide to document `s` and `/` together and forbids the retired
  `/` finder row.

## Report

The search and finder keys had settled in use, and the spellings they settled
on were not the ones registered. The finder should be reached from the
application namespace under one letter, and `/` should mean in-buffer search
the way it does in Vim and Helix.

### Observed bindings

Registered in `src/keymap.rs` and described in
`context/reference/helix-keymap-v1.md`:

| Sequence | Command |
| --- | --- |
| `s` | `search` — escaped literal, case-insensitive, in the current buffer |
| `S` | `search-regex` — regular expression in the current buffer |
| `/` | `open-file-picker`, registered as the alias of `Space / /` |
| `?` | unbound |
| `Space / /` | `open-file-picker` |
| `Space / s` | `global-search` |
| `Space / S` | `global-search-regex` |
| `Space / a` | `open-all-files-picker` |
| `Space / p` | `open-path-file-picker` |
| `Space g /` | `:git-search-commits` |

### Expected bindings

| Sequence | Command | Change |
| --- | --- | --- |
| `s` | `search` | unchanged |
| `/` | `search-regex` | takes the spelling `S` held |
| `Space f` | `open-file-picker` | the finder's short spelling, replacing bare `/` |
| `Space / f` | `open-file-picker` | the finder's namespace spelling, replacing `Space / /` |
| `Space / /` | `global-search-regex` | takes the spelling `Space / S` held |
| `Space / s` | `global-search` | unchanged |
| `Space / a` | `open-all-files-picker` | unchanged |
| `Space / p` | `open-path-file-picker` | unchanged |
| `Space g f` | `:git-search-commits` | replaces `Space g /` |
| `S` | — | unbound |
| `Space / S` | — | unbound |

The result is one rule per sigil rather than a set of independent choices.
`/` is in-buffer regular-expression search; the same key under the `Space /`
prefix widens the same flavour to the workspace, so `Space / s` mirrors `s` and
`Space / /` mirrors `/`. `f` means the finder in every namespace it appears
in: `Space f`, `Space / f`, and `Space g f`. This inverts the rationale
recorded for `Space g /`, which took `/` on the grounds that the sigil means
search in any namespace; that paragraph in
`context/reference/helix-keymap-v1.md` had to be rewritten rather than left
standing.

Roles follow the existing pattern: `Space / f` is the Primary binding and
`Space f` its Fast alias, the way `Space / /` carried the `/` alias.

`?` stays unbound. Search selects every match at once and lets `n` and `N`
supply a direction afterwards, which is why
`context/reference/helix-keymap-v1.md` records the key as deliberately unbound;
nothing in this change gives it a meaning it did not have.

### Naming

The user-facing name for this surface is the **Finder**. It was called a
picker, a file picker, and a project finder in different places, and the three
scope variants described themselves in three unrelated ways. Command
descriptions in `src/command.rs` should name it consistently, so that the same
word reaches the command palette, the key hints, and help:

- `open-file-picker` — the finder over the project;
- `open-all-files-picker` — the same finder, including ignored files;
- `open-path-file-picker` — the same finder rooted at a chosen path,
  including ignored files. For example: "Open the finder in a chosen path,
  including files Git ignores".

`context/reference/ui-vocabulary.md` already defines **project finder** as one
picker with two Tab-switched modes and a scan scope; that register keeps its
structural vocabulary. What changes is the user-visible prose: descriptions,
overlay titles, and help text say Finder rather than picker or file picker.
Colon spellings (`:file-picker`, `:file-picker-all`, `:file-picker-path`) are
command identities and stay as they are.

The `Space / p` path prompt's overlay title is covered by
`search_overlay_query_line.md`, which also renames it.

### Constraints

- `src/keymap.rs` remains the single declarative source of truth. Dispatch,
  help, and key hints must continue to read the same registry, so no surface
  can list an old spelling.
- `context/reference/helix-keymap-v1.md` is the register of record and must be
  updated in the same commit: the `s`, `S`, `?`, and `Space /` rows, the
  Search section's explanatory paragraphs, the single-letter binding audit,
  and the `Space g` row.
- `README.md` and the Search and Key bindings sections of
  `docs/user-guide.md` describe the current spellings and must move with them.
- Help prose in `src/help.rs` marks `/` and `s` as key bindings; the tutorial
  and any key hints embedded in prose must be checked for the retired keys.
- Retired sequences (`S`, `Space / S`, `Space g /`, bare `/` as the finder)
  become unbound rather than remaining registered as unsupported entries.

### Regression coverage

`tests/keymap.rs` already pinned several of the affected invariants and had to
be updated rather than relaxed:
`finder_and_workspace_search_are_global_in_every_buffer_scope`,
`character_find_and_project_find_keep_distinct_direct_bindings`,
`nested_space_tree_is_exact_primary_and_keeps_fast_compatibility_paths`,
`removed_duplicate_bindings_stay_unbound`, and
`every_mode_sequence_is_unique_and_described`. Coverage was to be added at the
behavior boundary for `Space f` resolving to the same command identity as
`Space / f` in every buffer scope, and for `S`, `Space / S`, `Space g /`, and
bare `/` as the finder reaching no binding.
