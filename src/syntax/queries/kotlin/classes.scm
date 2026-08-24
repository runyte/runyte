; SPDX-License-Identifier: MPL-2.0
; Runyte-authored class-like query for tree-sitter-kotlin-sg 0.4.1.

[
  (class_declaration)
  (object_declaration)
  (companion_object)
] @class.around

(class_declaration [(class_body) (enum_class_body)] @class.inside)
(object_declaration (class_body) @class.inside)
(companion_object (class_body) @class.inside)
