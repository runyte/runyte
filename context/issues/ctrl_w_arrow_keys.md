# Ctrl-w claims the arrow keys, so its hint popup cannot be scrolled with them

`Ctrl-w` is the only namespace whose key-hint popup cannot be scrolled with
Up and Down, because those keys are bound as pane motions underneath it. The
keys that do scroll it, `Ctrl-n` and `Ctrl-p`, are named in the popup's title
bar only as a fallback, and `Alt-j`/`Alt-k` are offered instead of the arrows.

## Observed behavior

`src/keymap.rs` registers `Ctrl-w Left/Down/Up/Right` as directional focus
alongside `Ctrl-w h/j/k/l` and their `Ctrl-`suffixed spellings, in Normal and
Select and in Insert and Replace. Global bindings reach a scoped view unless a
scoped binding shadows the same sequence, so the Insert-mode arrows are also
what the restricted `Ctrl-w` namespace answers to inside a terminal.

`KeyHintState::scrolls_with_arrow_in` (`src/key_hints.rs`) lets an arrow scroll
the popup only when appending it to the pending sequence reaches no binding.
Because `Ctrl-w Up` is a binding, the arrows are refused while `Ctrl-w` is
pending, and `draw_key_hints` writes `Ctrl-n/p Alt-j/k` in the title rather
than `Ctrl-n/p ↑/↓`.

`context/reference/helix-keymap-v1.md` records the current arrangement in two
places: the note that "`Alt-j` and `Alt-k` remain alternatives, including for
`z`, `Z`, and `Ctrl-w`, whose arrow continuations must still reach the
registry", and the `Ctrl-w h/j/k/l` row's "Ctrl-key and arrow suffix aliases
are also registered".

## Expected behavior

`Ctrl-w Up`, `Ctrl-w Down`, `Ctrl-w Left`, and `Ctrl-w Right` are unbound in
every mode. Directional focus keeps `Ctrl-w h/j/k/l` and the control-key
suffixes `Ctrl-w Ctrl-h/j/k/l`, which are the documented spellings. The
separate `Ctrl-h/j/k/l` keys behind `editor.fast_pane_keys` are unaffected.

With the arrows free, the `Ctrl-w` hint popup scrolls with Up and Down like
every other namespace, and its title advertises `↑/↓` through the existing
`scrolls_with_arrow_in` path rather than falling back to `Alt-j/k`.

Inside a terminal, an arrow after `Ctrl-w` then reaches no binding and cancels
the prefix, which leaves the source terminal in Insert as any other cancelled
`Ctrl-w` prefix does.

`z` and `Z` keep their arrow continuations; they are view-scroll commands
whose arrows mean what they say, and this issue does not touch them.

## Constraints

- The retired sequences become unbound rather than registered as unsupported
  entries, as `removed_duplicate_bindings_stay_unbound` in `tests/keymap.rs`
  expects of removed bindings.
- Both `context/reference/helix-keymap-v1.md` passages above must be rewritten
  in the same commit, including the Terminal Insert row that lists
  "control/arrow suffixes" among the restricted `Ctrl-w` continuations.
- `docs/user-guide.md` and the window and terminal help topics in
  `src/help.rs` describe the arrow spellings and must move with them.

## Regression coverage

Assert in `tests/keymap.rs` that the four sequences resolve to no binding in
Normal, Select, Insert, and Terminal Insert, and in `tests/key_hints.rs` that
Up and Down scroll the popup while `Ctrl-w` is pending and that its title
offers the arrows rather than `Alt-j/k`.
