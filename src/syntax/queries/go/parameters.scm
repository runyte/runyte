; SPDX-License-Identifier: MPL-2.0
; Runyte-authored parameter query for tree-sitter-go 0.25.0.
;
; One declaration can contain a grouped name list (`x, y int`). Keep that
; declaration together: splitting it would invent a type for only one name.

(parameter_list
  [
    (parameter_declaration)
    (variadic_parameter_declaration)
  ] @parameter.inside)
(parameter_list
  [
    (parameter_declaration)
    (variadic_parameter_declaration)
  ] @parameter.around
  .
  "," @parameter.around)
(parameter_list
  [
    (parameter_declaration)
    (variadic_parameter_declaration)
  ] @parameter.around
  .
  ")")
