---
title: "Ambiguous directional pane moves ignore recent focus"
status: resolved
reported: 2026-08-09
resolved: 2026-08-09
legacy_commit: b57c26b
---

## Resolution

Commit b57c26b (`Remember pane focus for directional moves`) fixed the directional focus choice. `App::focus` was ranking every pane on the requested shared edge by overlap length, center distance, and pane identity, so an ambiguous move always selected the same geometric winner and ignored where the user had just been.

The application now records pane activation and opening order at every pane-switching boundary. Directional focus still considers only panes sharing the requested edge, but among multiple candidates it selects the most recently active pane; when none has ever been active, it selects the most recently opened one. Closed panes are removed from the history.

Coverage lives in `src/app.rs`: `directional_focus_follows_shared_edges_in_a_nested_pane_grid` verifies that ambiguous moves follow changing activation history, and `ambiguous_directional_focus_falls_back_to_most_recently_opened_pane` verifies the unopened-candidate fallback.

## Report

With a grid of panes:

```
|    A     |
| B  |  C  |
```

moving from B to C, B to A, and C to A worked as expected. Moving down from A
always landed on C.

In such ambiguous cases the target should be the pane that was most recently
active, falling back to the most recently opened pane if none has ever been
active. The rule should hold for every grid: whenever a direction has more
than one candidate pane, the most recently active one wins.
