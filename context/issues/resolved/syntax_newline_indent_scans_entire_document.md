---
title: "Newline indentation scans unrelated document syntax"
status: resolved
reported: 2026-10-03
resolved: 2026-10-03
commit: b528dad
---

## Resolution

Commit `b528dad` (`perf(syntax): limit newline indentation to relevant tree ranges`).

DocumentSyntax::newline_indent formerly traversed the full tree for parse errors and ran its query over the entire document for every caret. Queries now cover the requested newline while retaining enclosing captures. A private shim uses the pinned Tree-sitter runtime subtree error bit to skip clean trees and branches; it still checks visible ERROR and missing nodes because hidden recovery nodes must not change existing diagnostics. The shim documents the Node ABI dependency and stays inside the syntax boundary.

The optimized answers match 7,872 differential decisions across valid, malformed, missing-token and injected-language fixtures. Warmed debug measurements excluding parsing changed from approximately 16 ms / 170 ms / 1.82 s at 20 KB / 200 KB / 2 MB to 12–25 microseconds. These are local scaling measurements, not optimized end-to-end latency claims.

Tests: `cached_parse_error_checks_match_visible_tree_walks` in `src/syntax/mod.rs`; `newline_indentation_retains_ancestor_captures_in_later_injections` and the manually invoked `newline_indentation_scaling` in `tests/syntax.rs`. The complete syntax unit and integration groups pass.

Known limitation: malformed trees can still require inspecting siblings along error-bearing branches. The private native accessor must be rechecked when tree-house-bindings changes its Node ABI.

## Report

`DocumentSyntax::newline_indent` walks every node in the selected parser layer
to detect parse errors, then runs the indentation query over the complete
document. Insert-mode Enter calls this method synchronously for each caret.
Unrelated functions elsewhere in a file therefore increase the cost of one
local indentation decision even though the parse tree is already available.

A fixture repeats `fn f() {\n    x();\n}\n` and queries the newline at offset 8.
With the current debug library, warmed exploratory measurements excluding
parsing are approximately 16 ms for 1,000 functions, 170 ms for 10,000, and
1.82 seconds for 100,000 functions. These correspond to 20 KB, 200 KB, and
2 MB of source. The measurements were collected during review and are evidence
of scaling, not a release-build input-to-frame benchmark.

Use the parser's existing error metadata and restrict indentation matching to
the queried newline while retaining enclosing indentation captures. Preserve
the distinction between ordinary parse errors and parser-inserted missing
nodes, injected-layer selection, source coordinates, explicit issue reporting,
and existing source-size and indentation-depth limits. Verification should
compare the optimized answers with the full-document implementation across
valid, malformed, missing-token, and injected-language fixtures, and measure
the same repeated-function workload before and after.
