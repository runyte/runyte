; SPDX-License-Identifier: MPL-2.0
; Runyte-authored function query for tree-sitter-lua 0.5.0.

[
  (function_declaration)
  (function_definition)
] @function.around

(function_declaration body: (block) @function.inside)
(function_definition body: (block) @function.inside)
