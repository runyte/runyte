---
title: "Alternate terminal screens hide retained history from the shared budget"
status: resolved
reported: 2026-10-03
resolved: 2026-10-03
commit: 17fe8c6
---

## Resolution

Commit `17fe8c6` (`fix(terminal): charge hidden primary history to the shared budget`).

TerminalSessions previously asked Emulator::grid for retained history, which selects an alternate screen without scrollback while its primary history remains allocated. The emulator now exposes primary-history accounting and eviction independently of the active grid. Budget enforcement still evicts review copies first, then the least-recently-active primary history, and advances revisions without changing live screen cells.

Coverage: alternate_screen_history_remains_charged_and_evictable in src/terminal/tests/read.rs verifies retained bytes, shared-budget eviction across terminals, unchanged alternate-screen reads, revision advancement, and lost-history reporting after returning to primary. It failed before the fix; all 16 terminal read tests pass.

## Report

The shared terminal retention budget omits a terminal's primary-screen
scrollback while its alternate screen is active. Both payload accounting and
eviction inspect only `Emulator::grid()`, which selects the alternate grid.
That grid has no history, but the primary grid and its scrollback remain
allocated. Several full-screen programs can therefore retain more history than
the workspace's 64 MiB terminal budget reports or enforces.

Retained primary scrollback must continue to count and remain eligible for
oldest-history eviction while an alternate screen is visible. Eviction must
leave both live screens intact, preserve active alternate-screen reads, advance
the conservative read revision, and report lost primary history when the
terminal returns to that screen.

Reproduce by producing primary-screen scrollback, entering the alternate screen
with `ESC [ ? 1049 h`, and comparing retained payload accounting before and
after. With a small injected workspace budget, output in another terminal
should evict the older retained rows even while the first terminal uses its
alternate screen.
