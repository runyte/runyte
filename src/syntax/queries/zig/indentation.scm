; SPDX-License-Identifier: MIT
; Adapted from tree-sitter-zig 1.1.2 queries/indents.scm for Runyte's
; bounded indentation capture dialect.
[
  (block)
  (switch_expression)
  (initializer_list)
] @indent.always
