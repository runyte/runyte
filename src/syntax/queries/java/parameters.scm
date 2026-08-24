; SPDX-License-Identifier: MPL-2.0
; Runyte-authored parameter query for tree-sitter-java 0.23.5.

(formal_parameters
  [(formal_parameter) (spread_parameter) (receiver_parameter)] @parameter.inside)

(formal_parameters
  [(formal_parameter) (spread_parameter) (receiver_parameter)] @parameter.around
  .
  "," @parameter.around)

(formal_parameters
  [(formal_parameter) (spread_parameter) (receiver_parameter)] @parameter.around
  .
  ")")

(lambda_expression parameters: (identifier) @parameter.inside)
(lambda_expression parameters: (identifier) @parameter.around)

(inferred_parameters
  (identifier) @parameter.inside)

(inferred_parameters
  (identifier) @parameter.around
  .
  "," @parameter.around)

(inferred_parameters
  (identifier) @parameter.around
  .
  ")")
