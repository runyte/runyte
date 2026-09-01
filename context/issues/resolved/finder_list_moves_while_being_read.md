---
title: "The finder list and its header move while a result is being read"
status: resolved
reported: 2026-09-01
resolved: 2026-09-01
commit: 2263f6c
---

## Resolution

Fixed by 2263f6c "Keep the finder list still while it is being read".

Two of the three changes are about a terminal, which was the reported
aggravating condition. `note_terminal_finder_change` marks a session changed
on every chunk its child writes, and the event loop reads those sessions back
on `FINDER_TERMINAL_REFRESH_INTERVAL`. In content mode that is proportionate:
the child's output *is* the corpus, and the refill machinery already drops the
rows it is about to read again and draws no frame in between. In name mode it
was not. A name-mode item is the session's title, command, and activity, so a
child that writes without renaming itself produces the item the finder already
holds — and `refresh_terminal_finder_item` ranked it regardless. Each of those
cost a rank of the whole file corpus through the background ranker, set
`picker.ranking`, and replaced every row in the list by way of
`apply_background_matches`. Once an interval, for the whole time a child is
writing, the list was rebuilt to say exactly what it already said, and the
header flipped to `scanning` and back on the way.

`ResourceItem` now derives equality and `ResourceFinder::terminal_item_differs`
asks whether a re-read says anything new before any of that happens.
`refresh_terminal_finder_item` reports whether it changed the list, and
`refresh_finder_terminals` returns that, so the event loop draws no frame for
a refresh that found nothing. Content mode is unchanged: there a refresh has
new rows by construction.

The interval itself goes from three seconds to five. The reporter offered
freezing terminal state at the moment the finder opens as the alternative;
the bounded refresh machinery already existed and reads only what a child has
added since, so the interval was widened rather than the freshness given up.

The third change is the guard described in
[finder_content_match_cannot_be_previewed](finder_content_match_cannot_be_previewed.md):
a finder file match is an index into one scan's entry table, and it is now
read only together with the scan that produced it.

Tests: `output_that_leaves_a_terminal_item_unchanged_does_not_move_the_name_list`
in `src/app/tests/search_and_pickers.rs` writes sixty-four rows that say
nothing new about the session, requires the refresh to report no change and
no row to move, and then requires a title the reader can search for to be
taken. `busy_terminal_updates_only_its_name_finder_item_and_selected_preview`
in the same file continues to cover the case where a burst does change the
item.

The header's counts were left updating at the frame rate in the change above
and were paced in a follow-up. `App::pace_picker_progress` runs once per
frame from `prepare_view`, and both header renderers read the value it holds
rather than the live corpus. It republishes at most once a second while work
is in flight, and at once when the work settles, because a settled header
reading something merely recent is wrong rather than slow. It covers the
plain picker's header as well as the finder's, since both show the same pair
and reading them from one place is what keeps the two renderers from
disagreeing about it. Its test is
`a_header_count_changes_once_a_second_and_publishes_what_the_work_settled_on`
in `src/app/tests/search_and_pickers.rs`, which drives the clock by rewinding
the published instant rather than by sleeping.

The `scanning`/`ready` word is stable for the duration of a scan, since
`picker.loading` covers it; what made it flicker after a scan settled was the
name-mode terminal rank above.

## Report

The project finder's list and its header change more than a reader can
follow.

Two observations. The overlay title flickers. And with a terminal open, the
result list is updated quickly enough to be distracting while reading it.

The report proposed either freezing terminal state at the moment the finder
is opened, or, if the machinery for bounded refreshes already existed,
bounding terminal re-reads to no more often than every five seconds.
