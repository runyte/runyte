---
title: "No command aligns the columns of a Markdown table"
status: resolved
reported: 2026-08-12
resolved: 2026-08-12
legacy_commit: 768b896
---

## Resolution

Fixed by `768b896`, "Add Space p t to align the columns of a selected table".

`src/table.rs` is a new module holding one pure function,
`format_table(text, tab_width) -> Option<String>`. `None` means the text is not
a table and is what produces the error the report asked for; identical output
means there was nothing to align. Nothing else in the module is public, so the
recognition rules stay in one place and the editor sees only the answer.

What makes a run of lines a table was the decision the rest followed from. Rows
open with `|` and divide their cells on unescaped `|`, and at least one
separator row of dashes has to be among them. The separator carries the whole
weight of detection: a run of pipe rows alone is as likely to be a Rust closure
(`|value| value + 1`) or a diff hunk, and accepting those would have reformatted
text nobody called a table while leaving the requested error with nothing to
catch. The separator is recognized wherever it falls rather than only as the
table's second line, because the selection is a hand-picked run of rows that may
open on the separator or close below a footer rule.

The report asked for `---+---` to survive, and the module generalizes that
rather than special-casing it: a separator row records the character at each
boundary position and rebuilds from that list, so `+---+---+` comes back with
its `+` signs, a row mixing the two keeps each character where it was, and a
separator narrower than the table repeats its last character to fill. GitHub's
`:---`, `:---:`, and `---:` colons survive on the row that carried them and also
decide whether each column's content sits left, centred, or right.

A tab inside a cell is expanded to spaces at `editor.tab_width` stops rather
than measured. A tab is as wide as its distance to the next stop, and the column
each cell lands on is exactly what the formatter is working out, so measuring
one would mean solving for it. Writing the spaces settles it: the row that comes
out holds no tab left to re-measure and is as wide in the pane as it was
computed to be. This is the one thing beyond spacing the command changes, and it
changes whitespace into whitespace, inside a cell whose surrounding whitespace
is trimmed regardless. Everything else is measured the way `wrap` and `ui`
measure it, one cell per character with the Unicode width where there is one, so
a formatted table lines up in the pane it was formatted in.

`App::format_selected_tables` in `src/app.rs` is the editor side. Alone among
the selection-wide text transforms it widens each span to whole rows first,
through `whole_line_span`: a table row is only a row from its opening `|` to its
close, so a selection landing mid-row would otherwise be read as prose and
refused, and `x` is not the only way people reach for a run of lines. Widening
never reaches past the last selected row, so a table continuing below the
selection stays as it was.

Widening can bring two ranges that did not overlap onto the same rows — two
selections on different columns of one row become one span — and
`Transaction::new` drops a change overlapping an earlier one, deliberately,
since two cursors editing one region is a selection-model bug rather than a
text-model one. `merged_line_spans` therefore folds the widened spans before
they become changes, rather than silently skipping rows the status line has just
reported as formatted. It folds spans divided by nothing but a single line
terminator too: every row between them is selected, so they are one run of rows,
and formatting them apart would give one table two sets of column widths. A
wider gap holds a row nobody selected, which stays outside the change and keeps
the two spans apart. Nothing is edited unless every selection holds a table,
because a partial success would leave no way to tell which selection the status
line was about.

The command sits at `Space p t`, joining wrapping and joining under a namespace
renamed to "Wrapping, joining, and tables", rather than opening a namespace of
its own for one command. It has no Helix equivalent;
`context/reference/helix-keymap-v1.md` carries its row.

Tests, all passing:

- `src/table.rs`: `format_table_pads_every_column_to_its_widest_cell` uses the
  table from the report; `format_table_keeps_the_characters_the_separator_was_drawn_with`
  covers `+---+---+` and a mixed row; `format_table_honours_alignment_colons`;
  `format_table_rejects_pipe_rows_with_no_separator_among_them` covers the
  closure shapes; `format_table_rejects_a_line_that_is_not_a_row`;
  `format_table_allows_blank_lines_around_the_table`;
  `format_table_expands_a_tab_inside_a_cell_to_the_configured_stops` at widths 4
  and 2; `format_table_measures_cells_the_way_the_pane_draws_them`;
  `format_table_squares_up_rows_that_disagree_on_column_count`;
  `format_table_keeps_indentation_and_carriage_returns`;
  `format_table_treats_an_escaped_pipe_as_content`;
  `format_table_pads_a_row_that_ends_without_a_boundary`;
  `format_table_leaves_an_already_formatted_table_untouched`.
- `src/app.rs`: `space_p_t_aligns_the_columns_of_the_selected_table` drives the
  report's table through real key dispatch and checks the single undo step;
  `space_p_t_keeps_a_separator_drawn_with_plus_signs`;
  `space_p_t_widens_a_selection_that_starts_and_ends_mid_row`;
  `space_p_t_folds_selections_that_widen_onto_the_same_rows` and
  `space_p_t_folds_selections_lying_on_consecutive_rows` cover the merge;
  `space_p_t_allows_blank_lines_inside_the_selection`;
  `space_p_t_refuses_a_selection_that_holds_no_table` and
  `space_p_t_refuses_pipe_rows_with_no_separator_among_them` cover the error;
  `space_p_t_expands_a_tab_in_a_cell_to_the_configured_tab_width`;
  `space_p_t_leaves_an_already_aligned_table_alone`;
  `space_p_t_formats_every_selection_as_one_transaction`.
- `tests/keymap.rs`: `space_p_t_exposes_format_table_in_normal_and_select_modes`.

Known limitation: a data row whose cells hold nothing but dashes, such as
`| - | - |`, reads as a separator and is rewritten as one, losing the dashes as
content. This is the cost of recognizing separators anywhere instead of tying
them to the table's second line, which the hand-picked selection ruled out.
Borderless tables, written without the outer `|`, are not detected at all:
accepting them would make any line containing a `|` a table row, and the report
left which other conventions matter undecided. Rows are formatted only relative
to each other, so nothing enforces a maximum table width.

## Report

There is no command for table formatting.

Markdown tables are often written as:

```
| Column 1 | Column 2 |
|---|---|
| Value | abc |
| Longer text | Very very long text |
```

Sometimes they are:

```
| Column 1 | Column 2 |
+---+---+
| Value | abc |
| Longer text | Very very long text |
```

There may be other conventions; which ones is left undecided.

Selecting the table lines and pressing `Space p t` should format the first table
like this:

```
| Column 1    | Column 2            |
|-------------|---------------------|
| Value       | abc                 |
| Longer text | Very very long text |
```

Special conventions such as using `---+---` instead of `---|---` are to be
respected.

If the selected text is not a proper table, the command should yield an error
saying that a table is not detected in the selected lines.

Selecting more lines than the table is to be allowed if the additional lines are
just whitespace and newlines.
