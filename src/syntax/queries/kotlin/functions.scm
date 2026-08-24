; SPDX-License-Identifier: MPL-2.0
; Runyte-authored function query for tree-sitter-kotlin-sg 0.4.1.

[
  (function_declaration)
  (anonymous_function)
  (secondary_constructor)
  (lambda_literal)
] @function.around

(function_declaration (function_body (_) @function.inside))
(anonymous_function (function_body (_) @function.inside))
(secondary_constructor (statements) @function.inside)
(lambda_literal (statements) @function.inside)
