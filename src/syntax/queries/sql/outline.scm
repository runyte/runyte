; SPDX-License-Identifier: MPL-2.0
; Runyte-authored document outline for tree-sitter-sequel 0.3.11.

(create_table
  (object_reference name: (identifier) @outline.name)) @outline.type
(create_view
  (object_reference name: (identifier) @outline.name)) @outline.type
(create_materialized_view
  (object_reference name: (identifier) @outline.name)) @outline.type
(create_function
  (object_reference name: (identifier) @outline.name)) @outline.function
