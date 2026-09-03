# `y` immediately followed by `p` appears to do nothing

Selecting text, yanking it with `y`, and pressing `p` leaves the buffer
exactly as it was. Nothing is inserted, the caret does not move, and the
status line reports no refusal, so the editor looks like it dropped the key.

## Reproduction

With `alpha bravo` on the only row and the caret at column 0:

1. `v l l l l` selects `alpha`.
2. `y` writes `alpha` to the unnamed register and returns Normal mode.
3. `p`.

The text is still `alpha bravo` and the primary range is still the one that
was yanked.

## Observed behavior

The two rules that meet here are each documented and each working as
specified in `context/reference/helix-keymap-v1.md`:

- `y` (`App::write_yanked_register`, `src/app/editing.rs`) returns Select mode
  to Normal but deliberately keeps the selection, "which is what lets `P`
  paste at its start".
- `p` (`App::paste_register`, `src/app/editing.rs`) replaces any range that
  holds text rather than pasting past it, because a selection-first editor has
  to be able to say "put this there instead".

Composed, `y p` replaces the yanked range with the register that was just
taken from it. The transaction is a replacement of a span by identical text,
so the visible result is unchanged. It is not free:

- `Buffer::apply` rejects only an empty transaction, so this one records an
  undo entry. `history_len` grows by one, and the `u` that follows also looks
  like it did nothing.
- The buffer is marked modified by an edit that changed no text.

The same shape appears wherever a yank leaves a range selected:

- `x y p` with a linewise register replaces the touched rows with themselves.
- `Space c y` then `Space c p` does the same through the system clipboard, and
  `clipboard_yank` (`src/app/search_history.rs`) does not even leave Select
  mode, so `p` there replaces a point range as well.

The report attributed this to `y` clearing the selection and leaving a bare
caret, in which case `p` would insert after the caret. That is not what
happens: the selection is retained, and retaining it is the cause.

## Expected behavior

After `y`, pressing `p` puts the yanked text into the buffer where the reader
can see it, rather than overwriting the source with a copy of itself. A paste
that cannot change anything must not consume an undo step or mark the buffer
modified.

## Points the fix has to settle

- **Which of the two rules gives.** Either `y` collapses the selection to a
  caret — which contradicts the documented reason it is kept, and breaks `y P`
  pasting at the start of what was yanked — or `p` stops treating the range a
  yank left behind as a replacement target, or a paste whose span already
  holds exactly the register text is refused. The third reading is narrow but
  behaves oddly for a reader deliberately replacing one occurrence of a word
  with an identical one.
- **Where the text lands and where the selection ends up.** If `y p` becomes
  an insertion, state whether it inserts after the selection's head, after its
  end, or on the next line for a linewise register, and what is selected
  afterwards.
- **`P` must keep working.** `y P` pasting at the start of the yanked range is
  documented behavior and existing coverage; whatever the fix does to the
  retained selection must not take it away.
- **Deliberate self-replacement.** `p` over a selection whose text happens to
  equal the register is a legitimate action after moving the caret elsewhere;
  the fix should distinguish that from the immediate `y p` sequence rather
  than suppressing both, or say plainly that it does not.
- **The clipboard pair.** `Space c y` leaves Select mode intact, so its
  behavior diverges from `y` before the fix is even considered. Decide whether
  the two are brought into line.
- **`Y`.** `yank-line` leaves the selection and caret alone by design. A `Y p`
  with a non-empty selection replaces that selection with the yanked lines,
  which does change the text but for the same reason. Say whether it is in
  scope.

## Constraints

- `context/reference/helix-keymap-v1.md` rows for `y`, `p`/`P`, and, if the
  fix touches them, `Y` and `Space c y/p/P`, must be rewritten in the same
  commit. Both of the rules above are stated there as deliberate deviations,
  so whichever one changes is a change to the register of record.
- `docs/user-guide.md` and the editing help topic in `src/help.rs` describe
  yank and paste and must move with it.
- A paste that produces no change must leave `Buffer::history_len` and the
  modified flag alone.

## Regression coverage

Cover in `src/app/tests/editing.rs`, beside
`p_replaces_a_selection_while_capital_p_still_pastes_beside_it`, which already
moves the caret between the yank and the paste and so does not see this: `y`
then `p` with no intervening motion inserts the register and leaves the buffer
changed; the undo history grows by exactly one entry that reverts it; `y` then
`P` still pastes at the start of the yanked range; the linewise `x y p` case;
and the clipboard pair through the test clipboard port.
