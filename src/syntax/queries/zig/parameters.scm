; SPDX-License-Identifier: MPL-2.0
; Runyte-authored parameter query for tree-sitter-zig 1.1.2.

(parameters (parameter) @parameter.inside)
(parameters
  (parameter) @parameter.around
  .
  "," @parameter.around)
(parameters
  (parameter) @parameter.around
  .
  ")")
