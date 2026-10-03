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
