; SPDX-License-Identifier: MPL-2.0
; Runyte-authored parameter query for tree-sitter-python 0.25.0.

(parameters (parameter) @parameter.inside)
(parameters
  (parameter) @parameter.around
  .
  "," @parameter.around)
(parameters
  (parameter) @parameter.around
  .
  ")")
