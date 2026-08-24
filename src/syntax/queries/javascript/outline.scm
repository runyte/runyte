; SPDX-License-Identifier: MPL-2.0
; Runyte-authored outline query for tree-sitter-javascript 0.25.0.

(class_declaration name: (_) @outline.name) @outline.class
(method_definition name: (_) @outline.name) @outline.method

[
  (function_declaration name: (identifier) @outline.name)
  (generator_function_declaration name: (identifier) @outline.name)
] @outline.function

(lexical_declaration
  (variable_declarator
    name: (identifier) @outline.name
    value: [(arrow_function) (function_expression) (generator_function)])) @outline.function

(lexical_declaration
  (variable_declarator
    name: (identifier) @outline.name
    value: (class))) @outline.class

(variable_declaration
  (variable_declarator
    name: (identifier) @outline.name
    value: [(arrow_function) (function_expression) (generator_function)])) @outline.function

(variable_declaration
  (variable_declarator
    name: (identifier) @outline.name
    value: (class))) @outline.class

(assignment_expression
  left: [(identifier) @outline.name
         (member_expression property: (property_identifier) @outline.name)]
  right: [(arrow_function) (function_expression) (generator_function)]) @outline.function

(assignment_expression
  left: [(identifier) @outline.name
         (member_expression property: (property_identifier) @outline.name)]
  right: (class)) @outline.class

(pair
  key: [(property_identifier) @outline.name
        (string (string_fragment) @outline.name)]
  value: [(arrow_function) (function_expression) (generator_function)]) @outline.function

(pair
  key: [(property_identifier) @outline.name
        (string (string_fragment) @outline.name)]
  value: (class)) @outline.class
