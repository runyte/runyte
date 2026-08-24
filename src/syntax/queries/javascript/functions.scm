; SPDX-License-Identifier: MPL-2.0
; Runyte-authored function query for tree-sitter-javascript 0.25.0.

[
  (function_declaration)
  (function_expression)
  (generator_function_declaration)
  (generator_function)
  (arrow_function)
  (method_definition)
] @function.around

(function_declaration body: (_) @function.inside)
(function_expression body: (_) @function.inside)
(generator_function_declaration body: (_) @function.inside)
(generator_function body: (_) @function.inside)
(arrow_function body: (_) @function.inside)
(method_definition body: (_) @function.inside)
