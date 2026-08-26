---
title: "Terminal emulation violated cell, mode, color, and erase semantics"
status: resolved
reported: 2026-08-26
resolved: 2026-08-26
commit: 57e6132
---

## Resolution

Commit 57e6132 (`Harden terminal cell and mode semantics`) corrected six
confirmed emulator defects. A one-column grid no longer retains an orphaned
wide lead, and disabling DECAWM no longer arms delayed wrap at the right
margin. IRM now shifts by the printed glyph's cell width after delayed wrap or
wide overflow has resolved the actual destination row and column.

Private modes 47, 1047, and 1049 now keep their distinct alternate-screen
semantics: 47 preserves without saving, 1047 preserves on entry and clears on
exit, and 1049 saves the primary cursor and pen while clearing on entry.
Screen-owned saved cursor state prevents an alternate-screen save from
corrupting the primary restore. Extended SGR colors require complete, exact
forms with channels representable as `u8`; malformed values are ignored
instead of wrapping or fabricating omitted channels. ED3 clears scrollback
only and leaves the live screen intact.

Coverage lives in `src/terminal/grid.rs` in
`a_one_column_grid_never_keeps_an_orphaned_wide_lead`,
`disabled_autowrap_overwrites_the_final_cell_without_pending_wrap`, and
`erase_display_three_clears_only_scrollback`. `src/terminal/emulator.rs` adds
`invalid_extended_colours_do_not_wrap_or_fabricate_channels`,
`alternate_screen_modes_keep_their_distinct_clear_and_save_semantics`,
`insert_mode_shifts_by_the_printed_glyph_width`,
`insert_mode_resolves_delayed_wrap_before_shifting`, and
`insert_mode_resolves_wide_overflow_before_shifting`.

## Report

Terminal escape parsing, emulation, the cell grid, scrollback, and terminal
key encoding required a focused hardening review with child-process output
treated as untrusted input. The scope included `src/terminal/parser.rs`,
`src/terminal/emulator.rs`, `src/terminal/grid.rs`, `src/terminal/keys.rs`, and
`tests/terminal.rs`.

The review covered incomplete, malformed, nested, and oversized escape
sequences; invalid UTF-8; control-string termination; parameter arithmetic;
cursor and region bounds; alternate screens; modes; resizing; scrollback
bounds; colors and attributes; combining and wide characters; grapheme
replacement; mouse and paste modes; and large or deliberately pathological
streams. Stream-boundary behavior was required to remain deterministic.
