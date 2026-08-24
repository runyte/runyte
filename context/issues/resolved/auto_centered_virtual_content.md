---
title: "Generated read-only pages could not follow the geometry of the pane showing them"
status: resolved
reported: 2026-08-13
resolved: 2026-08-15
legacy_commit: ff13c9b
---

## Resolution

Commit ff13c9b (`Centre generated pages against the pane rather than their own text`) added content alignment as a view capability and made `:about` its first caller.

The placement rule lives in the new `src/content_alignment.rs`. A `ContentAlignment` names a horizontal and a vertical option, each `Left`/`Top` or `Center`; a `ContentLayout` pairs one with the display width of the block it describes. Two saturating divisions turn a pane's geometry into an indent and a row offset, and nothing else in the module knows about buffers, panes, or drawing. The width is measured when the text is set rather than every frame, and re-measured by `Buffer::replace_virtual_text`, so a resize costs only the divisions. It is the width of the whole buffer rather than of the visible rows: measuring what is on screen would slide a page sideways as it scrolled.

Content is placed as one block. Each line is not centred on its own, because that would pull an ASCII logo apart; the whole page shifts together and its relative indentation survives. `about::render` was rewritten around that: it now emits a block per group — logo, version, description, tagline, heading, key table — each centred against the widest of them, so the page carries no margin of its own. The former fixed ten-cell `LEFT_MARGIN` is gone.

`Buffer` carries the layout, set through the consuming builder `Buffer::aligned` and read through `Buffer::content_layout`. Alignment is refused on an editable buffer, in the builder rather than at each call site: padding is only safe beside text a caret cannot reach, and in an editable buffer the drawn column and the stored column would disagree the moment anyone typed. `App::open_virtual_page` is how a generated view asks for it, and because the layout belongs to the buffer, reopening the page onto the one already there keeps it centred.

`prepare_view` subtracts the indent from `text_width` before anything else is measured against it, so wrapping, clipping, and row-hint placement all work in the width the text actually has, and `PreparedPane` carries `content_indent` for a frontend to draw. Vertical centring is deliberately narrower than the horizontal case: a page that asks for it is projected from its own first row, and if the whole of it fits it is held there with the leftover height split above and below. That is why scrolling a page that fits does nothing — there is nothing off-screen to reach — and why scrolling one that does not fit behaves exactly as it did before. The held-open rows are `PreparedRow::padding`, which reaches the snapshot as `SnapshotRow::Padding`: like a diff's filler it belongs to no line, but it stands for nothing rather than for a line the other side has, so a frontend draws it blank instead of hatched, and not as the marker for a row past the end.

The interaction model the report left open was not invented. What the fix commits to instead is the boundary that model will need: alignment never touches the buffer, so a row and a column mean the same thing at every pane size, and anything a producer anchors to them survives a resize. `App::pointer_offset` translates a click back through `content_indent` in the one place pointer hit-testing already lived, and a click landing in the margin names the first column of the row, as a click on the gutter already did.

Protocol `VERSION` moved to 7: a pane frame now carries `content_indent` and can contain padding rows, neither of which a host running the previous binary can serve.

Tests:

- `src/content_alignment.rs`: `a_centered_block_is_measured_at_its_widest_line`, `content_wider_or_taller_than_the_pane_starts_at_its_first_cell`, `wide_glyphs_are_measured_in_cells_rather_than_characters`, `an_unaligned_buffer_is_never_padded`, `each_axis_is_chosen_on_its_own`, `replacement_text_is_measured_again`
- `src/about.rs`: `about_is_a_block_with_no_margin_around_it`, `about_contains_the_source_logo_version_and_first_steps`
- `tests/content_alignment.rs`: `a_centered_page_is_drawn_at_the_middle_of_its_pane`, `resizing_the_pane_re_centres_the_page_without_rewriting_it`, `reopening_a_centered_page_keeps_its_alignment`, `a_centered_page_that_fits_is_held_down_the_pane`, `scrolling_a_page_that_fits_leaves_it_centred`, `a_centered_page_taller_than_the_pane_still_scrolls`, `two_panes_on_one_page_are_centred_independently`, `alignment_never_moves_anything_in_the_buffer`, `clicking_centred_text_lands_on_the_character_under_the_pointer`, `clicking_the_margin_names_the_start_of_the_row`, `only_a_page_that_asked_for_it_is_aligned`

Known limitation: the interaction model for focusable or activatable regions is not implemented, only made possible. Alignment is also whole-buffer: a page cannot centre one region and leave another against the left edge, which is why `about::render` still carries the relative offsets of its own blocks in its text. Content is measured in display cells with no tab expansion, so a generated page that indents with tabs would be centred against a width it is not drawn at.

## Report

Generated and interactive read-only buffers need presentation-level content alignment that follows the live pane geometry.

The immediate example is the `:about` page. Its text contained fixed leading spaces, so it could not stay centered when opened in differently sized panes or when a pane was resized. A reusable way was needed for a read-only buffer or view to describe content that should be centered automatically, without rewriting the underlying buffer text whenever its geometry changes.

This was to be a general view capability rather than an about-page special case. It also had to suit future interactive read-only buffers whose rendered content contains focusable or activatable "buttons". Keyboard and pointer activation should resolve against stable semantic identities rather than positions produced by padding, while ordinary read-only buffer behavior such as scrolling, searching, splitting, and closing remains available.

The exact representation, horizontal and vertical alignment options, and interaction model were left undecided.
