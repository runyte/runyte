; SPDX-License-Identifier: MPL-2.0
; Runyte-authored TypeScript outline additions for tree-sitter-typescript 0.23.2.

(abstract_class_declaration name: (type_identifier) @outline.name) @outline.class
(interface_declaration name: (type_identifier) @outline.name) @outline.interface
(module name: (_) @outline.name) @outline.module
(type_alias_declaration name: (type_identifier) @outline.name) @outline.type
(enum_declaration name: (identifier) @outline.name) @outline.type
(function_signature name: (identifier) @outline.name) @outline.function
(method_signature name: (_) @outline.name) @outline.method
(abstract_method_signature name: (_) @outline.name) @outline.method
