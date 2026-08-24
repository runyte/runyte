; SPDX-License-Identifier: MPL-2.0
; Runyte-authored class-like query for tree-sitter-java 0.23.5.

[
  (class_declaration)
  (interface_declaration)
  (enum_declaration)
  (record_declaration)
  (annotation_type_declaration)
] @class.around

(class_declaration body: (class_body) @class.inside)
(interface_declaration body: (interface_body) @class.inside)
(enum_declaration body: (enum_body) @class.inside)
(record_declaration body: (class_body) @class.inside)
(annotation_type_declaration body: (annotation_type_body) @class.inside)
