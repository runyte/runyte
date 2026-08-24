; SPDX-License-Identifier: MPL-2.0
; Runyte-authored function query for tree-sitter-bash 0.25.1.

(function_definition) @function.around
(function_definition body: (_) @function.inside)
