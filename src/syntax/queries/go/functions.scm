; SPDX-License-Identifier: MPL-2.0
; Runyte-authored function query for tree-sitter-go 0.25.0.

[
  (function_declaration)
  (method_declaration)
  (func_literal)
] @function.around

(function_declaration body: (block) @function.inside)
(method_declaration body: (block) @function.inside)
(func_literal body: (block) @function.inside)
