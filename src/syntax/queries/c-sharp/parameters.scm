; SPDX-License-Identifier: MPL-2.0
; Runyte-authored parameter query for tree-sitter-c-sharp 0.23.5.

(parameter_list (parameter) @parameter.inside)
(bracketed_parameter_list (parameter) @parameter.inside)

(parameter_list
  (parameter) @parameter.around
  .
  "," @parameter.around)
(parameter_list
  (parameter) @parameter.around
  .
  ")")

(bracketed_parameter_list
  (parameter) @parameter.around
  .
  "," @parameter.around)
(bracketed_parameter_list
  (parameter) @parameter.around
  .
  "]")
