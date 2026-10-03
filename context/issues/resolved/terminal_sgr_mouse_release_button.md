---
title: "SGR mouse releases lose the released button identity"
status: resolved
reported: 2026-10-03
resolved: 2026-10-03
commit: aa05cd0
---

## Resolution

Commit `aa05cd0` (`fix(terminal): retain button identity in SGR releases`).

TerminalSession::sgr_mouse_bytes used the legacy button-3 release marker inside SGR reports. Releases now preserve the original left, middle, or right button and modifier bits; the final lowercase m identifies release as required by SGR mode.

Coverage: sgr_mouse_encoding_preserves_coordinates_buttons_and_modifiers in src/terminal/mod.rs checks all three buttons on press and release, with and without modifier bits, and preserves coordinate and wheel coverage. The regression failed before the fix and passes afterward.

## Report

Integrated terminals encode every SGR mouse release as button `3`, regardless
of whether the released button is left, middle, or right. This is the legacy
X10 release marker. SGR mode (`DECSET 1006`) preserves the original button code
(`0`, `1`, or `2`) and uses the final lowercase `m` to distinguish release.
Programs using the SGR button identity cannot correctly pair these releases
with their preceding presses.

Release reports should retain the pressed button and modifier bits, with the
same one-based coordinates as press reports and a final `m`. For example, a
left release at column 5, row 3 must be `ESC [ < 0 ; 5 ; 3 m` rather than
`ESC [ < 3 ; 5 ; 3 m`.

The encoding contract is documented in the SGR (1006) section of
[XTerm Control Sequences](https://invisible-island.net/xterm/ctlseqs/ctlseqs.pdf).
Reproduce by enabling mouse tracking and SGR coordinates in an integrated
terminal, then inspecting its input bytes while pressing and releasing each
mouse button.
