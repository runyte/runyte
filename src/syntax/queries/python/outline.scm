; SPDX-License-Identifier: MPL-2.0
; Runyte-authored outline query for tree-sitter-python 0.25.0.

(class_definition name: (identifier) @outline.name) @outline.class

(decorated_definition
  definition: (class_definition
    name: (identifier) @outline.name)) @outline.class

(class_definition
  body: (block
    (function_definition name: (identifier) @outline.name) @outline.method))

(function_definition name: (identifier) @outline.name) @outline.function

(decorated_definition
  definition: (function_definition
    name: (identifier) @outline.name)) @outline.function

(module
  (expression_statement
    (assignment left: (identifier) @outline.name) @outline.constant))
