; SPDX-License-Identifier: MPL-2.0
; Runyte-authored function query for tree-sitter-c-sharp 0.23.5.

[
  (method_declaration)
  (local_function_statement)
  (constructor_declaration)
  (destructor_declaration)
  (operator_declaration)
  (conversion_operator_declaration)
  (anonymous_method_expression)
  (lambda_expression)
] @function.around

(method_declaration body: (_) @function.inside)
(local_function_statement body: (_) @function.inside)
(constructor_declaration body: (_) @function.inside)
(destructor_declaration body: (_) @function.inside)
(operator_declaration body: (_) @function.inside)
(conversion_operator_declaration body: (_) @function.inside)
(anonymous_method_expression (block) @function.inside)
(lambda_expression body: (_) @function.inside)
