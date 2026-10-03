---
title: "URL punctuation trimming repeatedly rescans candidate text"
status: resolved
reported: 2026-10-03
resolved: 2026-10-03
commit: 2a5cde6
---

## Resolution

Commit `2a5cde6` (`perf(navigation): index URL punctuation trimming across candidates`).

under_cursor counts delimiter totals once and indexes the trailing punctuation run. It subtracts consumed prefixes while scanning candidates and locates the first balanced closing delimiter with three indexed lookups. This handles both a long unmatched suffix and repeated URL prefixes when the caret lies on excluded punctuation. Control-character and empty-authority rejection avoid repeated whole-suffix validation, while balanced URL delimiters and literal path fallback retain their behavior.

Coverage: inferred_links_trim_long_mixed_suffixes_and_preserve_balanced_delimiters, punctuation_carets_try_later_prefixes_without_repeated_suffix_scans, and indexed_url_suffixes_match_character_by_character_trimming in src/navigation_target/tests.rs. All eight navigation tests pass, including differential comparison over 4,096 mixed suffixes and multiple prefix forms. benchmarks/navigation_target.py compiles actual old/current source and compares equal results. Three-sample medians: 65,536 unmatched closers improved 4879.092 ms to 0.127053 ms; 2,048 repeated prefixes improved 25.000171 ms to 0.067461 ms. These are adversarial scaling measurements, not ordinary navigation latency.

## Report

Inferring a web link under the cursor repeatedly counts opening and closing
delimiters across the entire remaining URL while trimming unmatched trailing
parentheses, brackets or braces. In `navigation_target::under_cursor`, each
trailing `)` invokes two `str::matches(...).count()` scans before removing one
character. A row containing `https://example.test/` followed by many unmatched
closing parentheses consequently takes quadratic time when `gf` infers the
target on the editor thread. Terminal review uses the same inference helper.

The punctuation trimming should preserve its current treatment of balanced
URL delimiters and prose punctuation while scanning each candidate a bounded
number of times. Regression coverage should include long unmatched suffixes,
balanced and nested delimiters, mixed punctuation, and a cursor outside the
retained URL. A stress comparison should measure the actual helper with
increasing suffix lengths rather than using a wall-clock assertion in a test.
