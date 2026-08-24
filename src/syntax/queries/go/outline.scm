; SPDX-License-Identifier: MPL-2.0
; Runyte-authored outline query for tree-sitter-go 0.25.0.

(package_clause
  (package_identifier) @outline.name) @outline.module

; The broad type entry is replaced by the more-specific struct/interface
; classification at the owned outline boundary when both match one target.
(type_spec
  name: (type_identifier) @outline.name) @outline.type

(type_spec
  name: (type_identifier) @outline.name
  type: (struct_type)) @outline.struct

(type_spec
  name: (type_identifier) @outline.name
  type: (interface_type)) @outline.interface

(type_alias
  name: (type_identifier) @outline.name) @outline.alias

(function_declaration
  name: (identifier) @outline.name) @outline.function

(method_declaration
  name: (field_identifier) @outline.name) @outline.method

(const_spec
  (identifier) @outline.name) @outline.constant
