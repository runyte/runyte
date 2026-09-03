# Finder and in-buffer search key bindings

The search and finder keys have settled in use, and the spellings they settled
on are not the ones currently registered. The finder should be reached from
the application namespace under one letter, and `/` should mean in-buffer
search the way it does in Vim and Helix.

## Observed bindings

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

## Expected bindings

| Sequence | Command | Change |
| --- | --- | --- |
| `s` | `search` | unchanged |
| `/` | `search-regex` | takes the spelling `S` holds today |
| `Space f` | `open-file-picker` | the finder's short spelling, replacing bare `/` |
| `Space / f` | `open-file-picker` | the finder's namespace spelling, replacing `Space / /` |
| `Space / /` | `global-search-regex` | takes the spelling `Space / S` holds today |
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
currently recorded for `Space g /`, which took `/` on the grounds that the
sigil means search in any namespace; that paragraph in
`context/reference/helix-keymap-v1.md` has to be rewritten rather than left
standing.

Roles follow the existing pattern: `Space / f` is the Primary binding and
`Space f` its Fast alias, the way `Space / /` carries the `/` alias today.

`?` stays unbound. Search selects every match at once and lets `n` and `N`
supply a direction afterwards, which is why
`context/reference/helix-keymap-v1.md` records the key as deliberately unbound;
nothing in this change gives it a meaning it did not have.

## Naming

The user-facing name for this surface is the **Finder**. It is currently
called a picker, a file picker, and a project finder in different places, and
the three scope variants describe themselves in three unrelated ways.
Command descriptions in `src/command.rs` should name it consistently, so that
the same word reaches the command palette, the key hints, and help:

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

## Constraints

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

## Regression coverage

`tests/keymap.rs` already pins several of the affected invariants and must be
updated rather than relaxed:
`finder_and_workspace_search_are_global_in_every_buffer_scope`,
`character_find_and_project_find_keep_distinct_direct_bindings`,
`nested_space_tree_is_exact_primary_and_keeps_fast_compatibility_paths`,
`removed_duplicate_bindings_stay_unbound`, and
`every_mode_sequence_is_unique_and_described`. Add coverage at the behavior
boundary for `Space f` resolving to the same command identity as
`Space / f` in every buffer scope, and for `S`, `Space / S`, `Space g /`, and
bare `/` as the finder reaching no binding.
