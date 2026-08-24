; SPDX-License-Identifier: MPL-2.0
; Runyte-authored document outline for tree-sitter-java 0.23.5.

(module_declaration name: (_) @outline.name) @outline.module

(class_declaration name: (identifier) @outline.name) @outline.class
(record_declaration name: (identifier) @outline.name) @outline.class
(interface_declaration name: (identifier) @outline.name) @outline.interface
(annotation_type_declaration name: (identifier) @outline.name) @outline.interface
(enum_declaration name: (identifier) @outline.name) @outline.enum

(method_declaration name: (identifier) @outline.name) @outline.method
(constructor_declaration name: (identifier) @outline.name) @outline.method
(compact_constructor_declaration name: (identifier) @outline.name) @outline.method
