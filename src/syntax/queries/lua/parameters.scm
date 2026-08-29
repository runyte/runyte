; SPDX-License-Identifier: MPL-2.0
; Runyte-authored parameter query for tree-sitter-lua 0.5.0.

(parameters
  [(identifier) (vararg_expression)] @parameter.inside)

(parameters
  [(identifier) (vararg_expression)] @parameter.around
  .
  "," @parameter.around)

(parameters
  [(identifier) (vararg_expression)] @parameter.around
  .
  ")")
