; SPDX-License-Identifier: MPL-2.0
; Runyte-authored class-like item query for tree-sitter-rust 0.24.2.

[
  (struct_item)
  (enum_item)
  (union_item)
  (trait_item)
  (impl_item)
] @class.around

(struct_item body: (field_declaration_list) @class.inside)
(enum_item body: (enum_variant_list) @class.inside)
(union_item body: (field_declaration_list) @class.inside)
(trait_item body: (declaration_list) @class.inside)
(impl_item body: (declaration_list) @class.inside)
