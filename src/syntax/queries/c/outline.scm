; SPDX-License-Identifier: MPL-2.0
; Runyte-authored outline query for tree-sitter-c 0.24.2.

(function_definition
  declarator: (function_declarator
    declarator: (identifier) @outline.name)) @outline.function

(function_definition
  declarator: (pointer_declarator
    declarator: (function_declarator
      declarator: (identifier) @outline.name))) @outline.function

(declaration
  declarator: (function_declarator
    declarator: (identifier) @outline.name)) @outline.function

(declaration
  declarator: (pointer_declarator
    declarator: (function_declarator
      declarator: (identifier) @outline.name))) @outline.function

(struct_specifier name: (type_identifier) @outline.name body: (_)) @outline.type
(union_specifier name: (type_identifier) @outline.name body: (_)) @outline.type
(enum_specifier name: (type_identifier) @outline.name body: (_)) @outline.type
(type_definition declarator: (type_identifier) @outline.name) @outline.type
(type_definition
  declarator: (pointer_declarator
    declarator: (type_identifier) @outline.name)) @outline.type
