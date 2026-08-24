---
title: "Directional pane movement skips an adjacent pane in a grid"
status: resolved
reported: 2026-08-09
resolved: 2026-08-09
legacy_commit: 91ba3bb
---

## Resolution

Commit `91ba3bb` ("Navigate panes by shared edges") changes `App::focus` from
choosing the nearest pane center in a direction to choosing panes that share
the requested boundary with a nonzero overlap. Candidate neighbors are ranked
by the span of their shared edge, then alignment, so horizontal and vertical
movements follow the visible pane grid rather than jumping through a pane
above or beside it.

`directional_focus_follows_shared_edges_in_a_nested_pane_grid` in `src/app.rs`
covers movement between both lower panes and upward movement from each to the
pane above.

## Report

With a grid of panes:

```
|------ Pane A --------|
|-- Pane B -|- Pane C -|
```

moving from B to C with `Ctrl-w l` (or `Ctrl-w Right`) landed on A instead,
requiring a second keystroke to reach C.

Pane movement should understand the grid — A is above both B and C, B is left
of C, C is right of B — so that directional motions behave intuitively.
