; SPDX-License-Identifier: MPL-2.0
; Runyte-authored class query for tree-sitter-javascript 0.25.0.

[
  (class_declaration)
  (class)
] @class.around

(class_declaration body: (class_body) @class.inside)
(class body: (class_body) @class.inside)
