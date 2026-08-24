; SPDX-License-Identifier: MPL-2.0
; Runyte-authored parameter query for tree-sitter-javascript 0.25.0.

(formal_parameters (_) @parameter.inside)
(formal_parameters
  (_) @parameter.around
  .
  "," @parameter.around)
(formal_parameters
  (_) @parameter.around
  .
  ")")

; Arrow functions may omit parentheses around one parameter.
(arrow_function parameter: (_) @parameter.inside)
(arrow_function parameter: (_) @parameter.around)
