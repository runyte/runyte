---
title: "Long Markdown marker runs cause repeated unbounded scans"
status: resolved
reported: 2026-10-03
resolved: 2026-10-03
commit: 57db4ce
---

## Resolution

Commit `57db4ce` (`perf(markdown): skip unparseable marker prefixes in one pass`).

The inline parser now emits the impossible-to-close prefix of oversized marker runs as literal text in one step, preserving the bounded tail for existing span parsing. Source-link navigation applies the same rule to backticks. Closing backtick inspection is bounded and fixed-width delimiter checks count only the required markers, preventing oversized input from bypassing the existing lookahead policy.

Coverage: long_inline_marker_runs_remain_literal_and_keep_trailing_spans and long_closing_backticks_do_not_hide_a_later_source_link in src/markdown.rs exercise 32,768-character runs of all four marker types, valid trailing spans, source/page position mappings, long closers, and subsequent link navigation. All 33 Markdown tests pass; tests assert behavior without timing thresholds.

## Report

Markdown's inline parser documents a 2,048-character lookahead limit, but
opening backtick, emphasis and strikethrough runs are counted without that
bound. When a long run does not form a span, rendering advances one character
and recounts the rest of the run. A paragraph containing `prefix ` followed
by tens of thousands of backticks, stars, underscores or tildes therefore
takes quadratic time on the editor thread. Source-link navigation has the
same behavior for unmatched backtick runs. Closing backtick runs can also
extend an otherwise bounded scan arbitrarily far.

Treat the part of an opening marker run that cannot possibly close within
the existing lookahead policy as literal text in one step. Preserve the
remaining suffix so a shorter valid span at its end still renders as before,
including its source-to-page offsets. Bound closing-run inspection without
changing the treatment of ordinary code, emphasis, links or escaped markers.
Tests should cover all four marker kinds, a shorter valid trailing span,
source-link navigation and long closing runs without timing assertions.
