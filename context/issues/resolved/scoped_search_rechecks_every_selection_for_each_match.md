---
title: "Scoped search rescans every selection for each match"
status: resolved
reported: 2026-10-03
resolved: 2026-10-03
commit: 297414c
---

## Resolution

Commit `297414c` (`perf(search): advance through selection scopes once per scan`).

text_matches sorts region bounds once and advances a region cursor as matches arrive in document order. The furthest end among regions beginning before the match is enough to test containment in one individual region; regions are not unioned, so overlapping selections cannot admit a match neither contains. Unicode and zero-width match handling remain unchanged.

Coverage: scoped_matching_preserves_individual_regions_with_unordered_and_overlapping_bounds and scoped_matching_handles_many_disjoint_selections in src/app/tests/search_region_cost.rs cover an oracle comparison, empty scopes, Unicode, overlapping and unordered spans, zero-width expressions, and 40,000 matches over 20,000 scopes. Both pass. benchmarks/scoped_search.py compiles actual before/current functions; the measured optimized median at 20,000 regions was 84.25 ms before and 4.18 ms after. This measures scoped matching rather than complete interactive search latency.

## Report

`text_matches` checks every candidate search match against the region list with
`spans.iter().any(...)`. A search started over thousands of selections therefore
performs quadratic containment work when the number of matches scales with the
number of regions. Both live search previews and accepted searches use this
path, so narrowing the results of a broad earlier search can stall input.

Reproduce with a buffer containing 20,000 copies of `aa `, select each `aa`, and
search for `a` inside the selections. The 40,000 candidate matches repeatedly
scan the region prefix even though matches and normal selections are ordered.

Search containment should advance through region boundaries once while matching
in document order. Preserve the requirement that one individual region contain
the entire match; joining adjacent or overlapping regions must not allow a match
that no single original selection contained. Preserve Unicode offsets,
zero-width regular-expression matches, and empty-region behavior.
