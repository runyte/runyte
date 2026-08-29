; SPDX-License-Identifier: MPL-2.0
; Runyte-authored document outline for tree-sitter-lua 0.5.0.

(function_declaration
  name: (identifier) @outline.name) @outline.function
(function_declaration
  name: (dot_index_expression field: (identifier) @outline.name)) @outline.function
(function_declaration
  name: (method_index_expression method: (identifier) @outline.name)) @outline.method

(assignment_statement
  (variable_list . (identifier) @outline.name)
  (expression_list . (function_definition))) @outline.function
(assignment_statement
  (variable_list . (dot_index_expression field: (identifier) @outline.name))
  (expression_list . (function_definition))) @outline.function
