; SPDX-License-Identifier: MPL-2.0
; Runyte-authored indentation query for tree-sitter-lua 0.5.0.
[
  (function_definition)
  (function_declaration)
  (if_statement)
  (for_statement)
  (repeat_statement)
  (while_statement)
  (table_constructor)
  (do_statement)
] @indent.always
(arguments) @indent.begin
