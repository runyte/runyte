; SPDX-License-Identifier: MPL-2.0
; Runyte-authored captures for tree-sitter-elixir 0.3.5.
; The packaged query uses @module (unmapped) and @string.special.symbol;
; these later captures give aliases and atoms their existing theme scopes.

(alias) @namespace
[(atom) (quoted_atom) (keyword) (quoted_keyword)] @constant
