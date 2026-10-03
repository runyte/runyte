---
title: "LSP location picker titles duplicate their initial letter"
status: resolved
reported: 2026-10-03
resolved: 2026-10-03
commit: f2e6818
---

## Resolution

Commit `f2e6818` (`fix(lsp): capitalize location picker titles once`).

show_locations previously prepended the capitalized initial to the full label, yielding Rreferences or Ddefinition. It now appends only the remainder of the label and constructs the picker directly.

Coverage: a_single_goto_result_moves_the_caret_and_several_open_a_picker in src/app/tests/language.rs verifies References alongside the existing navigation behavior. The regression failed before the change and passes afterward.

## Report

The picker shown for multiple language-server locations duplicates its title's
initial letter. A references response opens a picker titled `Rreferences`, and
multiple definition results produce `Ddefinition`.

The title should capitalize the request label once, producing `References` or
`Definition`. The location rows and navigation behavior are otherwise unchanged.

Reproduce by requesting references for a symbol whose server returns at least
two locations and inspecting the resulting picker title.
