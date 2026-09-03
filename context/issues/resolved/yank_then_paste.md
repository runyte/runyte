---
title: "`y` immediately followed by `p` appears to do nothing"
status: resolved
reported: 2026-09-03
resolved: 2026-09-03
commit: 2596fde
---

## Resolution

Commit `2596fde` (`Collapse the selection a yank was taken from`) changed
`App::write_yanked_register` (`src/app/editing.rs`), which returned Select
mode to Normal but deliberately kept the range it had just copied.

Two separately correct rules composed into a dead key. `y` kept the
selection so that `P` could paste at its start; `p` replaces any range that
holds text, because Helix's `replace_with_yanked` binding is spent on
Replace mode here and a selection-first editor still has to be able to say
"put this there instead". Pressing `p` straight after `y` therefore replaced
the yanked characters with the register taken from them. The text was
unchanged, but the edit was not free: `Buffer::apply` rejects only an empty
transaction, so a replacement of a span by identical text still recorded an
undo entry and marked the buffer modified. The retained range was also
invisible while it did this — `snapshot.rs` skips a lone Runyte range in
Normal mode rather than drawing it as selected — so nothing on screen
explained why the key did nothing.

`write_yanked_register` now collapses the selection before handing back to
Normal, keeping one caret per range at the head of each. The caret therefore
sits on the last character copied, which is what makes `y` then `p` paste
past the yanked text and read as the duplicate the sequence looks like.
A half-open range ends one past its last character, so a `HalfOpen` or
`VimLinewise` selection is converted with `vim_half_open_to_inclusive` before
collapsing; without that step the caret after a Vim `y y` landed at the start
of the following row and the linewise `p` inserted below the wrong line. The
same idiom already exists in `collapse_vim_normal_selection`
(`src/app/input.rs`).

`Y` shares the path and follows, deliberately: its previous contract of
leaving the selection and caret alone had the same invisible-range problem,
and having the two yank keys end a gesture differently would be harder to
explain than the loss.

This is a deviation from Helix, where `y` keeps the selection, and it retires
the documented behavior of `P` pasting at the start of what was just yanked:
`P` now pastes before the caret the yank left. The `y`, `Y`, `p`/`P`, and
`Space c y/p/P` rows of `context/reference/helix-keymap-v1.md` and the
editing section of `docs/user-guide.md` were rewritten with the change.

The report attributed the symptom to `y` clearing the selection and leaving a
bare cursor. It did not: the selection was retained, and retaining it was the
cause. The fix makes the report's premise true rather than treating it as an
observation.

Coverage lives in `src/app/tests/editing.rs` as
`a_yank_collapses_its_selection_so_the_paste_after_it_duplicates`,
`capital_p_after_a_yank_pastes_before_the_last_character_yanked`,
`a_yank_leaves_one_caret_per_range`,
`a_transient_line_yank_leaves_a_caret_and_pastes_the_line_below`, and
`a_transient_line_yank_pastes_above_the_caret_it_left_with_capital_p`; the Vim
grammar's `y y` then `p` is covered by
`vim_operator_counts_linewise_registers_change_and_cw_are_shared_edits` in
`src/app/tests/commands.rs`, and the character-position case by
`yank_paste_and_prompt_editing_use_unicode_character_positions` in
`src/app/tests/search_and_pickers.rs`.

Known limitation: `Space c y` is unchanged and still leaves the selection
intact in Select mode, so `Space c y` followed by `Space c p` continues to
replace the copied text with itself. `ClipboardYank` is the same command a
right click runs, and a copy made with the mouse must not destroy the
selection it copied; separating the two paths was out of scope for this fix.

## Report

Selecting text, yanking it with `y`, and pressing `p` left the buffer exactly
as it was. Nothing was inserted, the caret did not move, and the status line
reported no refusal, so the editor looked like it had dropped the key.

### Reproduction

With `alpha bravo` on the only row and the caret at column 0:

1. `v l l l l` selects `alpha`.
2. `y` writes `alpha` to the unnamed register and returns Normal mode.
3. `p`.

The text was still `alpha bravo` and the primary range was still the one that
had been yanked.

### Observed behavior

The two rules that met here were each documented in
`context/reference/helix-keymap-v1.md` and each working as specified:

- `y` (`App::write_yanked_register`, `src/app/editing.rs`) returned Select
  mode to Normal but deliberately kept the selection, "which is what lets `P`
  paste at its start".
- `p` (`App::paste_register`, `src/app/editing.rs`) replaces any range that
  holds text rather than pasting past it, because a selection-first editor
  has to be able to say "put this there instead".

Composed, `y p` replaced the yanked range with the register that had just
been taken from it. The transaction was a replacement of a span by identical
text, so the visible result was unchanged. It was not free:

- `Buffer::apply` rejects only an empty transaction, so this one recorded an
  undo entry. `history_len` grew by one, and the `u` that followed also
  looked like it did nothing.
- The buffer was marked modified by an edit that changed no text.

The same shape appeared wherever a yank left a range selected:

- `x y p` with a linewise register replaced the touched rows with themselves.
- `Space c y` then `Space c p` did the same through the system clipboard, and
  `clipboard_yank` (`src/app/search_history.rs`) does not even leave Select
  mode, so `p` there replaced a point range as well.

The report attributed this to `y` clearing the selection and leaving a bare
caret, in which case `p` would insert after the caret. That is not what
happened: the selection was retained, and retaining it was the cause.

### Expected behavior

After `y`, pressing `p` puts the yanked text into the buffer where the reader
can see it, rather than overwriting the source with a copy of itself. A paste
that cannot change anything must not consume an undo step or mark the buffer
modified.

### Points the fix had to settle

- **Which of the two rules gives.** Either `y` collapses the selection to a
  caret — which contradicts the documented reason it was kept, and changes
  where `y P` pastes — or `p` stops treating the range a yank left behind as
  a replacement target, or a paste whose span already holds exactly the
  register text is refused.
- **Where the text lands and where the selection ends up**, including for a
  linewise register.
- **`P` must keep working.** `P` pasting at the start of the yanked range was
  documented behavior with existing coverage.
- **Deliberate self-replacement.** `p` over a selection whose text happens to
  equal the register is a legitimate action after moving the caret elsewhere,
  and the fix had to distinguish that from the immediate `y p` sequence
  rather than suppressing both.
- **The clipboard pair.** `Space c y` left Select mode intact, so its
  behavior diverged from `y` before the fix was considered.
- **`Y`.** `yank-line` left the selection and caret alone by design. `Y p`
  with a non-empty selection replaced that selection with the yanked lines,
  which does change the text but for the same reason.
