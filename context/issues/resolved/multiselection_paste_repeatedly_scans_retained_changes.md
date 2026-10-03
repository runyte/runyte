---
title: "Multi-selection paste repeatedly scans retained replacements"
status: resolved
reported: 2026-10-03
resolved: 2026-10-03
commit: c1b0aa5
---

## Resolution

Commit `c1b0aa5` (`perf(editing): locate retained paste changes by ordered bounds`).

paste_register now locates the retained normalized replacement by binary partition over its ordered bounds. The existing containment predicate and selection reconstruction remain intact, including overlapping linewise spans and zero-length replacements; each selection no longer scans every earlier change.

Coverage: pasting_many_search_matches_selects_each_unicode_replacement_and_undoes_once in src/app/tests/paste_many_selections.rs verifies 20,000 Unicode replacements, every resulting selection, primary selection ownership, and one-step undo. All 106 app editing tests also pass. An exploratory debug comparison improved the full stress paste from 1.126 s to 0.658 s with indexed transaction mapping in both versions; this is not a release-build latency claim.

## Report

After constructing a paste transaction, `paste_register` searches its complete
change list from the beginning for each replaced selection. It needs the retained
change because linewise expansion can make disjoint selections overlap, causing
transaction normalization to drop duplicate replacements. With N retained
replacements, finding them again performs quadratic work even when transaction
offset mapping itself is indexed.

Reproduce by selecting 20,000 separate one-character matches and pasting a
two-character register over all of them. Every replacement selection scans all
earlier transaction changes before it can select the inserted text.

Locate retained changes using their ordered boundaries. Preserve selection of
the actual retained replacement when original spans overlap, repeated insertions
share an offset, or an empty final line produces a zero-length replacement. Keep
the primary selection and single-step undo behavior unchanged.
