; SPDX-License-Identifier: MPL-2.0
; Runyte-authored outline query for tree-sitter-swift 0.7.3.

(class_declaration "class" name: (type_identifier) @outline.name) @outline.class
(class_declaration "struct" name: (type_identifier) @outline.name) @outline.struct
(class_declaration "enum" name: (type_identifier) @outline.name) @outline.enum
(class_declaration "actor" name: (type_identifier) @outline.name) @outline.actor
(class_declaration "extension" name: (_) @outline.name) @outline.extension
(protocol_declaration name: (type_identifier) @outline.name) @outline.interface

(class_declaration
  (class_body
    (function_declaration name: (simple_identifier) @outline.name) @outline.method))

(function_declaration name: (simple_identifier) @outline.name) @outline.function
(protocol_function_declaration name: (simple_identifier) @outline.name) @outline.method
(init_declaration "init" @outline.name) @outline.method
(deinit_declaration "deinit" @outline.name) @outline.method
(source_file
  (property_declaration
    (pattern (simple_identifier) @outline.name)) @outline.property)

(class_body
  (property_declaration
    (pattern (simple_identifier) @outline.name)) @outline.property)

(typealias_declaration name: (type_identifier) @outline.name) @outline.alias
(subscript_declaration "subscript" @outline.name) @outline.subscript
