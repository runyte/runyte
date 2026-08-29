; SPDX-License-Identifier: MPL-2.0
; Runyte-authored document outline for tree-sitter-c-sharp 0.23.5.

(namespace_declaration name: (_) @outline.name) @outline.module
(file_scoped_namespace_declaration name: (_) @outline.name) @outline.module

(class_declaration name: (identifier) @outline.name) @outline.class
(struct_declaration name: (identifier) @outline.name) @outline.struct
(interface_declaration name: (identifier) @outline.name) @outline.interface
(enum_declaration name: (identifier) @outline.name) @outline.enum
(record_declaration name: (identifier) @outline.name) @outline.class
(delegate_declaration name: (identifier) @outline.name) @outline.type

(method_declaration name: (identifier) @outline.name) @outline.method
(local_function_statement name: (identifier) @outline.name) @outline.function
(constructor_declaration name: (identifier) @outline.name) @outline.method
(destructor_declaration name: (identifier) @outline.name) @outline.method
(property_declaration name: (identifier) @outline.name) @outline.property
