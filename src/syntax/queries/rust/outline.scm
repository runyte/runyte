; SPDX-License-Identifier: MPL-2.0
; Runyte-authored outline query for tree-sitter-rust 0.24.2.

(mod_item name: (identifier) @outline.name) @outline.module

[
  (struct_item name: (type_identifier) @outline.name)
  (enum_item name: (type_identifier) @outline.name)
  (union_item name: (type_identifier) @outline.name)
  (type_item name: (type_identifier) @outline.name)
] @outline.type

(trait_item name: (type_identifier) @outline.name) @outline.interface

(impl_item
  body: (declaration_list
    (function_item name: (identifier) @outline.name) @outline.method))

(trait_item
  body: (declaration_list
    [
      (function_item name: (identifier) @outline.name)
      (function_signature_item name: (identifier) @outline.name)
    ] @outline.method))

(function_item name: (identifier) @outline.name) @outline.function
(function_signature_item name: (identifier) @outline.name) @outline.function

(const_item name: (identifier) @outline.name) @outline.constant
(static_item name: (identifier) @outline.name) @outline.constant
(macro_definition name: (identifier) @outline.name) @outline.macro
