; SPDX-License-Identifier: MPL-2.0
; Runyte-authored document outline for tree-sitter-zig 1.1.2.

(function_declaration name: (identifier) @outline.name) @outline.function

(variable_declaration
  (identifier) @outline.name
  (struct_declaration)) @outline.struct
(variable_declaration
  (identifier) @outline.name
  (union_declaration)) @outline.struct
(variable_declaration
  (identifier) @outline.name
  (opaque_declaration)) @outline.struct
(variable_declaration
  (identifier) @outline.name
  (enum_declaration)) @outline.enum
(variable_declaration
  (identifier) @outline.name
  (error_set_declaration)) @outline.enum
