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
