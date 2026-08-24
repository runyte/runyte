; SPDX-License-Identifier: MPL-2.0
; Runyte-authored class-like query for tree-sitter-go 0.25.0.

(type_spec type: [(struct_type) (interface_type)]) @class.around

(type_spec
  type: (struct_type
    (field_declaration_list) @class.inside))

(type_spec
  type: (interface_type
    [
      (method_elem)
      (type_elem)
    ]+ @class.inside))
