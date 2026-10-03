---
title: "Syntax bracket matching invents partners in incomplete or quoted text"
status: resolved
reported: 2026-10-03
resolved: 2026-10-03
commit: b6ed7d1
---

## Resolution

Commit `b6ed7d1` (`fix(syntax): require real paired delimiter edges for bracket jumps`).

DocumentSyntax::matching_bracket previously recognized only one character at a container edge, without proving that the caret was at that edge or that the opposite edge was its delimiter. It now requires both paired characters, an exact caret edge, and actual non-missing anonymous delimiter children. Incomplete containers cannot borrow an inner closing bracket, and a brace inside a string or comment cannot jump into its enclosing block.

Coverage: match_bracket_requires_a_present_partner_in_the_same_container, match_bracket_ignores_a_caret_inside_strings_and_comments, and match_bracket_preserves_pairs_in_nested_and_injected_syntax in tests/syntax.rs cover malformed and valid JSON/Rust/HTML/Markdown, Unicode offsets and both directions. The defect regressions failed before the fix; all 124 syntax integration tests pass.

## Report

`DocumentSyntax::matching_bracket` treats the opposite edge of an enclosing
syntax node as a bracket partner without checking its character or confirming
that the caret is the corresponding node edge. In JSON `[123`, matching the
opening `[` returns the final digit `3`; in `123]`, matching `]` returns `1`.
In Rust `fn main() { let s = "a { b"; }`, matching the brace inside the string
can jump to the outer block's closing brace. The same problem affects brackets
inside comments. The user-facing command is `mm`.

Matching must identify an actual pair anchored at the caret. Missing delimiter
tokens, mismatched node edges, nested unterminated containers, and extra closing
delimiters must not borrow a partner from another node. Valid structural pairs
must keep working in both directions, including nested containers and injected
languages. Brackets in strings and comments must not redirect to enclosing
syntax. The check must retain character-offset behavior for non-ASCII source.
