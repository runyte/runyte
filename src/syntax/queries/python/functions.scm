; SPDX-License-Identifier: MPL-2.0
; Runyte-authored function query for tree-sitter-python 0.25.0.

(function_definition) @function.around
(function_definition body: (block) @function.inside)
