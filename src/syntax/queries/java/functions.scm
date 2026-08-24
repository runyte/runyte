; SPDX-License-Identifier: MPL-2.0
; Runyte-authored function query for tree-sitter-java 0.23.5.

[
  (method_declaration)
  (constructor_declaration)
  (compact_constructor_declaration)
  (lambda_expression)
] @function.around

(method_declaration body: (block) @function.inside)
(constructor_declaration body: (constructor_body) @function.inside)
(compact_constructor_declaration body: (block) @function.inside)
(lambda_expression body: (_) @function.inside)
