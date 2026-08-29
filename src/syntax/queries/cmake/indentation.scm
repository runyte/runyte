; SPDX-License-Identifier: MIT
; Adapted from tree-sitter-cmake 0.7.4 queries/indents.scm for Runyte's
; bounded indentation capture dialect.
[
  (if_condition)
  (foreach_loop)
  (while_loop)
  (function_def)
  (macro_def)
  (block_def)
] @indent.always
(normal_command) @indent.begin
