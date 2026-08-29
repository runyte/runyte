; SPDX-License-Identifier: MPL-2.0
; Runyte-authored class-like query for tree-sitter-c-sharp 0.23.5.

[
  (class_declaration)
  (struct_declaration)
  (interface_declaration)
  (enum_declaration)
  (record_declaration)
] @class.around

(class_declaration body: (_) @class.inside)
(struct_declaration body: (_) @class.inside)
(interface_declaration body: (_) @class.inside)
(enum_declaration body: (_) @class.inside)
(record_declaration body: (_) @class.inside)
