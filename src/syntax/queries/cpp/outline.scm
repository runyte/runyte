; SPDX-License-Identifier: MPL-2.0
; Runyte-authored C++ additions for tree-sitter-cpp 0.23.4.

(class_specifier name: (type_identifier) @outline.name body: (_)) @outline.class
(alias_declaration name: (type_identifier) @outline.name) @outline.alias
(concept_definition name: (identifier) @outline.name) @outline.concept

(function_definition
  declarator: (function_declarator
    declarator: (field_identifier) @outline.name)) @outline.method

(function_definition
  declarator: (function_declarator
    declarator: (qualified_identifier
      name: (identifier) @outline.name))) @outline.method

(namespace_definition name: (namespace_identifier) @outline.name) @outline.module

; Constructors, destructors, and operators use declarator kinds which the C
; fragment deliberately cannot name.
(field_declaration_list
  (function_definition
    declarator: (function_declarator
      declarator: [
        (identifier) @outline.name
        (destructor_name) @outline.name
        (operator_name) @outline.name
      ])) @outline.method)

(function_definition
  declarator: (function_declarator
    declarator: (qualified_identifier
      name: [
        (destructor_name) @outline.name
        (operator_name) @outline.name
      ]))) @outline.method

; A template is one declaration in the outline. Capturing the wrapper keeps
; attributes and template parameters in the item range while the target stays
; on the declared name.
(template_declaration
  (function_definition
    declarator: (function_declarator
      declarator: [
        (identifier) @outline.name
        (field_identifier) @outline.name
        (operator_name) @outline.name
      ]))) @outline.function

(template_declaration
  (function_definition
    declarator: (pointer_declarator
      declarator: (function_declarator
        declarator: (identifier) @outline.name)))) @outline.function

(template_declaration
  (alias_declaration name: (type_identifier) @outline.name)) @outline.alias

(template_declaration
  (concept_definition name: (identifier) @outline.name)) @outline.concept
