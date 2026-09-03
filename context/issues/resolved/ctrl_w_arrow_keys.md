---
title: "Ctrl-w pane arrows prevent arrow scrolling in the key-hint popup"
status: resolved
reported: 2026-09-03
resolved: 2026-09-03
commit: 70e9277
---

## Resolution

Commit `70e9277` (`Free Ctrl-w arrows for hint scrolling`) removed the eight
arrow-suffix entries that `built_in_bindings` registered for the modal and
Insert/Replace `Ctrl-w` namespaces. `KeyHintState::scrolls_with_arrow_in` was
correctly reserving an arrow whenever appending it to the pending sequence
found a binding, so those compatibility aliases made the `Ctrl-w` popup the
only modal namespace popup that could not use Up and Down for scrolling.

Directional pane focus retains `Ctrl-w h/j/k/l` and
`Ctrl-w Ctrl-h/j/k/l`. The removed arrow sequences are absent from the
registry rather than represented by unsupported bindings, which lets the
existing Normal and Select key-hint path consume Up and Down and advertise
`↑/↓`. Insert and Replace, including Terminal Insert, show the pending prefix
on the interaction line rather than drawing the popup; an unbound continuation
cancels it. The former `Alt-j`/`Alt-k` popup fallback was retired so visible
completion, list, and key-hint controls share the conventional `Ctrl-n` and
`Ctrl-p` pair. This deliberately retires four compatibility spellings; `z`
and `Z` keep their arrow continuations because those arrows are view-scroll
commands.

Tests covering the behavior are
`ctrl_w_arrow_suffixes_stay_unbound_in_every_mode_and_terminal_insert` in
`tests/keymap.rs` and
`ctrl_w_popup_scrolls_with_arrows_and_advertises_them` in
`tests/key_hints.rs`. The
`alt_j_and_k_cancel_insert_ctrl_w_instead_of_scrolling` test in that file
covers the retired fallback in Insert, while
`popup_with_bound_arrows_advertises_only_control_scroll` in the same file
continues to cover the retained `z` behavior.

## Report

`Ctrl-w` was the only namespace whose key-hint popup could not be scrolled
with Up and Down, because those keys were bound as pane motions underneath
it. The keys that did scroll it, `Ctrl-n` and `Ctrl-p`, were named in the
popup's title bar only as a fallback, and `Alt-j`/`Alt-k` were offered instead
of the arrows.

`src/keymap.rs` registered `Ctrl-w Left/Down/Up/Right` as directional focus
alongside `Ctrl-w h/j/k/l` and their `Ctrl-`-suffixed spellings, in Normal and
Select and in Insert and Replace. Global bindings reach a scoped view unless
a scoped binding shadows the same sequence, so the Insert-mode arrows were
also what the restricted `Ctrl-w` namespace answered to inside a terminal.

`KeyHintState::scrolls_with_arrow_in` in `src/key_hints.rs` lets an arrow
scroll the popup only when appending it to the pending sequence reaches no
binding. Because `Ctrl-w Up` was a binding, the arrows were refused while
`Ctrl-w` was pending, and `draw_key_hints` wrote `Ctrl-n/p Alt-j/k` in the
title rather than `Ctrl-n/p ↑/↓`.

The expected behavior was for `Ctrl-w Up`, `Ctrl-w Down`, `Ctrl-w Left`, and
`Ctrl-w Right` to be unbound in every mode. Directional focus was to keep
`Ctrl-w h/j/k/l` and the control-key suffixes `Ctrl-w Ctrl-h/j/k/l`, which are
the documented spellings. The separate `Ctrl-h/j/k/l` keys behind
`editor.fast_pane_keys` were unaffected.

With the arrows free, the modal `Ctrl-w` hint popup scrolls with Up and Down
like every other namespace and its title advertises `↑/↓` through the existing
`scrolls_with_arrow_in` path rather than falling back to `Alt-j/k`.

Inside a terminal, an arrow after `Ctrl-w` reaches no binding and cancels the
prefix, which leaves the source terminal in Insert as any other canceled
`Ctrl-w` prefix does. The retired sequences remain unbound rather than being
registered as unsupported entries.

`z` and `Z` keep their arrow continuations. They are view-scroll commands
whose arrows mean what they say, and were outside the scope of this change.
