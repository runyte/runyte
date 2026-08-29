; SPDX-License-Identifier: MPL-2.0
; Runyte-authored class-like container query for tree-sitter-zig 1.1.2.

(variable_declaration
  [(struct_declaration) (enum_declaration) (union_declaration) (opaque_declaration)]
    @class.inside) @class.around
