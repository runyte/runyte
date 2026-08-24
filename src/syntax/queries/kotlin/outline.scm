; SPDX-License-Identifier: MPL-2.0
; Runyte-authored document outline for tree-sitter-kotlin-sg 0.4.1.

(class_declaration "interface" (type_identifier) @outline.name) @outline.interface
(class_declaration "enum" (type_identifier) @outline.name) @outline.enum
(class_declaration "class" (type_identifier) @outline.name) @outline.class
(object_declaration (type_identifier) @outline.name) @outline.class
(companion_object (type_identifier) @outline.name) @outline.class
(type_alias (type_identifier) @outline.name) @outline.alias

(function_declaration (simple_identifier) @outline.name) @outline.function
(class_body
  (function_declaration (simple_identifier) @outline.name) @outline.method)
(enum_class_body
  (function_declaration (simple_identifier) @outline.name) @outline.method)
(class_body
  (secondary_constructor "constructor" @outline.name) @outline.method)
(enum_class_body
  (secondary_constructor "constructor" @outline.name) @outline.method)

(source_file
  (property_declaration
    (variable_declaration (simple_identifier) @outline.name)) @outline.property)
(class_body
  (property_declaration
    (variable_declaration (simple_identifier) @outline.name)) @outline.property)
(enum_class_body
  (property_declaration
    (variable_declaration (simple_identifier) @outline.name)) @outline.property)

(enum_entry (simple_identifier) @outline.name) @outline.constant
