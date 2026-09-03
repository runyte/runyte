---
title: "The path completion border clipped the keys out of its own title"
status: resolved
reported: 2026-09-03
resolved: 2026-09-03
commit: 952c1c2
---

## Resolution

Fixed in commit `952c1c2`, "Keep the path hint keys out of the border's way".

`path_completion_area` in `src/ui.rs` sizes this overlay from its content
rather than from a share of the editor, and for a list of short names the
title is the widest thing in it. It measured that title as the title text
plus the action hints, but `draw_snapshot_overlay` writes three segments
between the corners, not two: the title, then the list position when the
list is longer than the box shows, then the keys. The box was therefore
built exactly as wide as the title it does not carry, and Ratatui clipped
what did not fit — always the tail, which is the keys.

The width budget now includes that counter. It is measured in the same
`" · {first}–{last}/{total}"` shape the border writes, with the total
standing in for all three numbers: every number in the segment is at most
the total, so this is the segment at its widest and the border does not
move as the selection scrolls the numbers through their digit counts. The
counter is charged only when the border would write one, under the same
condition the drawing code uses — the list is longer than the rows on
screen.

Nothing else changed. The sibling `confirmation_overlay_area` has no rows
and so never carries a counter, and every other overlay takes a share of
the editor area rather than measuring its own title, so none of them was
sized short in this way.

Coverage is
`the_assistance_title_keeps_its_keys_when_it_also_counts_the_rows` in
`tests/path_completion.rs`, which types an absolute base into `:cd` over a
directory of twenty-two short names — short rows, so the title governs the
width — and asserts the border carries both the count and `Tab complete`.

Known limitation: the width is still only a budget for what the border
writes. Where the editor itself is narrower than the full title, the box
is clamped to the editor width and the tail is clipped as before, because
there is nowhere else for it to go.

## Report

The path completion assistance opened by a path-argument command drew a
title whose key hints were cut off. Typing `:cd <path>/` in a directory
holding more entries than the twelve rows the box shows produced:

```
┌ Choose path for :cd · 1–12/23 · ↑/↓ select · Tab┐
```

The border ends immediately after `Tab`, so `complete` — the word saying
what that key does — was not drawn. The expected title is the one the
overlay describes for itself:

```
┌ Choose path for :cd · 1–12/23 · ↑/↓ select · Tab complete ┐
```

The clipping depended on the rows. It appeared where the offered names
were short and the typed spelling was absolute, so no row carried a
resolved path in its detail column and the title was the widest content
in the box. Where a row was wider than the title, the box was already
wide enough and the whole title was drawn.
