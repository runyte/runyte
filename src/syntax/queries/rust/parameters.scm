; SPDX-License-Identifier: MPL-2.0
; Runyte-authored parameter query for tree-sitter-rust 0.24.2.

(parameters
  [
    (parameter)
    (self_parameter)
    (variadic_parameter)
  ] @parameter.inside)
(parameters
  [
    (parameter)
    (self_parameter)
    (variadic_parameter)
  ] @parameter.around
  .
  "," @parameter.around)
(parameters
  [
    (parameter)
    (self_parameter)
    (variadic_parameter)
  ] @parameter.around
  .
  ")")
