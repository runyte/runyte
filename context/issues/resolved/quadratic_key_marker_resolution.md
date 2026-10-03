---
title: "Key-marker resolution repeatedly scans the growing page"
status: resolved
reported: 2026-10-03
resolved: 2026-10-03
commit: ff5e663
---

## Resolution

Commit `ff5e663` (`perf(help): track key marker output offsets incrementally`)
keeps an output character counter in `key_spelling::resolve_with_map` instead
of recounting the entire accumulated string for every literal input character.
Literal characters, escaped braces and substituted key spellings each advance
that counter by their Unicode scalar count. The source-to-output boundary map
and substitution ranges retain their existing character-offset semantics.
The removed repeated prefix scans caused quadratic work even on plain prose;
this change makes that path linear without changing authored help content.

All seven `key_spelling` tests passed, including
`unicode_marker_offsets_and_escaped_boundaries_stay_exact` and
`large_unmarked_page_has_identity_character_offsets` in `src/key_spelling.rs`.
The parent reviewer independently checked all output append and offset-map
update paths. No wall-clock speedup is claimed for this change.

## Report

Generating the manual, tutorial, or contextual help runs
`key_spelling::resolve_with_map` over the authored text. The resolver recounts
all Unicode characters in the accumulated output at least twice for every
literal input character. Resolving a long page therefore performs quadratic
work on the editor thread even when most of the page has no key markers.
Plugin-supplied help is escaped and passes through the same resolver.

The required source-to-rendered offset map uses character offsets rather than
bytes. Maintain its output position incrementally as literal characters,
escaped braces, and key substitutions are appended. Preserve exact text,
substitution ranges, and every map boundary, including multibyte characters
and combining marks. A large plain-text regression should cover the formerly
quadratic path without a flaky wall-clock assertion.
