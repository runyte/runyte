---
title: "Background syntax work could be lost or retain the wrong parser variant"
status: resolved
reported: 2026-08-26
resolved: 2026-08-26
commit: c181cd4
---

## Resolution

Commit c181cd4 (`Keep background syntax updates per buffer`) replaced the
worker's process-wide request and event watch slots with per-buffer latest
requests and non-lossy completed events. A typing burst still coalesces for
one buffer, but work and results for distinct buffers can no longer overwrite
one another. Revision checks remain the final stale-result boundary.

`DocumentSyntax::update` now compares the parser language required by the new
document size with the active root language. Crossing the 128-KiB injection
limit in either direction performs a full parse with the correct injected or
plain variant before advancing the revision. Stale highlighting now returns
an empty result for empty or reversed ranges instead of passing reversed
bounds to `clamp`.

Coverage lives in `src/syntax/background.rs` in
`distinct_buffer_requests_are_not_coalesced_away` and
`reversed_stale_highlight_ranges_are_empty`, in `src/syntax/mod.rs` in
`updates_switch_parser_variants_across_the_injection_limit`, and in
`tests/background_syntax.rs` in
`late_tree_is_rejected_and_the_latest_coalesced_revision_applies` and
`stale_tree_exposes_translated_spans_but_no_structure_until_drain`.

## Report

Language detection, Tree-sitter parsing, incremental edits, highlighting,
structural queries, folds, and the background parse worker required a focused
hardening review. The scope included `src/syntax/`,
`src/app/syntax_workflows.rs`, syntax-driven editing callers, and syntax tests.

The review covered byte and character coordinate conversion, incremental edit
construction, revision identity, stale result rejection, pending-edit
translation, cancellation and replacement of queued work, parser and query
failures, malformed trees, injection limits, grammar detection, large files,
deep or adversarial syntax, fold-range validity, highlight clipping, and
behavior while no current tree is available. Existing fidelity and asynchrony
limits were to be preserved unless a confirmed defect required narrowing them.
