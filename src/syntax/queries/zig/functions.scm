; SPDX-License-Identifier: MPL-2.0
; Runyte-authored function query for tree-sitter-zig 1.1.2.

(function_declaration) @function.around
(function_declaration body: (block) @function.inside)
