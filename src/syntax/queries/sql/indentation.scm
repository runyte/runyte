; SPDX-License-Identifier: MIT
; Adapted from tree-sitter-sequel 0.3.11 queries/indents.scm for Runyte's
; bounded indentation capture dialect.
[
  (select)
  (cte)
  (column_definitions)
  (case)
  (subquery)
  (insert)
  (block)
] @indent.always
