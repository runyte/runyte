---
title: "Path completion hides matches in large directories"
status: resolved
reported: 2026-08-24
resolved: 2026-08-24
commit: 14bffec
---

## Resolution

Commit 14bffec (`Fix path completion in large directories`) fixed
`App::path_completion` and `App::matching_path_hints`, which stopped reading a
directory after the first 512 entries returned by `fs::read_dir` and only then
applied the typed prefix. Since filesystem enumeration order is unrelated to
the prefix, a matching name outside that arbitrary slice could not reach
either completion surface.

Both paths now match the prefix against the complete listing before retaining
the first 512 candidates in their visible order. Path completion rebuilds that
bounded candidate set as the prefix changes instead of narrowing the earlier
slice, so names remain reachable even when they were not among the rows shown
for a shorter prefix.

`DirectoryListings` was added to keep the full read off subsequent keystrokes
and command-palette redraws. It retains a small most-recently-used set,
invalidates a listing when the containing directory's modification time
changes or becomes unavailable, and gives an unsettled listing a two-second
volatile reuse window before reading it again. The most recent listing remains
cached even when it exceeds the ordinary aggregate entry bound, because that
is the listing for which another synchronous read would be most expensive.
Symlink-derived directory kinds are revalidated whenever a cached listing is
reused because changes to a link target do not update the containing
directory's modification time.

A later presentation fix corrected `ui::draw_snapshot_overlay` for attached
persistent clients. The renderer sized an anchored completion for its
candidate rows and borders, then drew the typed query in an additional
interior row. That displaced one candidate: three matches rendered as two,
and narrowing to one match left only the query visible. Anchored overlays now
include their query and message rows in the requested height, so the semantic
candidate set and the visible rows agree.

The completion behavior is covered by
`tests/path_completion.rs::a_wide_directory_offers_every_name_typed_into_it`,
`tests/path_completion.rs::a_wide_directory_offers_every_name_typed_into_the_palette`,
`tests/path_completion.rs::typing_further_into_a_wide_directory_narrows_rather_than_empties`,
`tests/path_completion.rs::a_deep_tree_completes_at_every_level`,
`tests/path_completion.rs::hidden_entries_stay_hidden_until_a_dot_is_typed`,
`tests/path_completion.rs::names_that_are_prefixes_of_each_other_all_stay_reachable`,
`tests/path_completion.rs::a_wide_directory_of_unicode_names_completes_on_the_typed_prefix`,
`tests/path_completion.rs::a_symlinked_directory_is_offered_as_a_directory`, and
`tests/path_completion.rs::the_bounded_rows_are_the_ones_the_full_listing_would_lead_with`.
The release budgets are covered by
`tests/performance.rs::completing_a_path_in_a_wide_directory_stays_within_budget`
and `tests/performance.rs::palette_path_rows_redraw_within_budget`. Cache
reuse, invalidation, bounds, unavailable paths, and changing symlink targets
are covered by the unit tests in `src/directory_listing.rs`, including
`a_directory_touched_moments_ago_is_reused_for_a_bounded_window`,
`a_change_inside_the_window_is_still_noticed`,
`a_directory_that_disappears_does_not_return_its_kept_listing`,
`a_listing_larger_than_the_entry_bound_is_still_kept`, and
`a_symlink_that_gains_or_loses_its_target_is_reclassified`.
Attached-client presentation of a path completion narrowed to one row is
covered by `src/ui.rs::attached_completion_keeps_a_row_below_its_query`.

Known limitation: the first completion request for a directory still reads
the complete directory synchronously, and the visible result remains bounded
to 512 rows. On a filesystem whose modification-time granularity cannot prove
that a just-written directory is stable, a cached listing may be stale for at
most the two-second volatile window.

## Report

Path completion offers no matches in a directory with many entries. The
behavior was observed while typing a path in insert mode: the popup that
opens on `/` lists entries, and the first character typed after it empties
the popup. Typing the next `/` makes entries appear again for a moment, and
the following character empties it once more. The command palette's rows for
a path argument behave the same way.

Small directories complete correctly, so the failure depends on the number of
entries rather than on the path being typed.

### Observed cause

Both path popups bound the filesystem work one keystroke may do, and both
applied that bound to the directory read rather than to what they keep:

- `App::path_completion` collected the first 512 entries `fs::read_dir`
  returned and offered them to the typed fragment afterwards.
- `App::matching_path_hints` did the same with the first 512 entries before
  comparing them against the typed prefix.

A directory read returns names in whatever order the filesystem holds them,
which for a large directory is neither sorted nor related to what is being
typed. In a directory of 20,000 files, the retained 512 are about 2.5% of it,
so a typed name is almost never among them and the popup shows nothing.

### Expected behavior

A name that exists in the directory is offered when it is typed, whatever the
size of the directory and wherever the filesystem returns it. A bound on how
many rows are shown is expected; a bound that decides which matches exist is
not.

### Constraints

The read happens on the input thread, between the keystroke and the redraw
that answers it, so completing a path in a very large directory must stay
within a keystroke. The command palette recomputes its rows for every frame
it draws, so its cost is paid again on each redraw.

### Reproduction

Create a directory with 20,000 files:

```sh
mkdir -p /tmp/wide && cd /tmp/wide
for i in $(seq -w 0 19999); do : > "file_$i.txt"; done
```

In insert mode, type `/tmp/wide/` — entries appear — then type `file_0`. The
popup empties, though 10,000 entries match that prefix. The same happens with
`:open /tmp/wide/file_0`.
