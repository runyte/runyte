; SPDX-License-Identifier: MIT
; Adapted from tree-sitter-ruby 0.23.1 queries/locals.scm.
; Tree-house requires the definition capture to name its highlight scope.
; Copyright (c) 2016 Rob Rix

; Methods isolate their parameters and body together. Tree-house scopes cover
; whole nodes, so a singleton method's receiver is also inside this boundary:
; references to outer locals in that receiver can use function highlighting.
([
  (method)
  (singleton_method)
] @local.scope
 (#set! local.scope-inherits false))

; Class/module headers are evaluated in the enclosing scope; only their
; bodies introduce a fresh local scope.
([
  (class body: (body_statement) @local.scope)
  (singleton_class body: (body_statement) @local.scope)
  (module body: (body_statement) @local.scope)
] (#set! local.scope-inherits false))

[
  (lambda)
  (block)
  (do_block)
] @local.scope

(block_parameter (identifier) @local.definition.variable)
(block_parameters (identifier) @local.definition.variable)
(destructured_parameter (identifier) @local.definition.variable)
(hash_splat_parameter (identifier) @local.definition.variable)
(lambda_parameters (identifier) @local.definition.variable)
(method_parameters (identifier) @local.definition.variable)
(splat_parameter (identifier) @local.definition.variable)

(keyword_parameter name: (identifier) @local.definition.variable)
(optional_parameter name: (identifier) @local.definition.variable)

(identifier) @local.reference

(assignment left: (identifier) @local.definition.variable)
(operator_assignment left: (identifier) @local.definition.variable)
(left_assignment_list (identifier) @local.definition.variable)
(rest_assignment (identifier) @local.definition.variable)
(destructured_left_assignment (identifier) @local.definition.variable)
(for pattern: (identifier) @local.definition.variable)
(exception_variable (identifier) @local.definition.variable)

; Pattern matching binds bare names, including nested destructuring, but a
; pinned name (^name) is a reference to an existing local, not a definition.
(in_clause pattern: (identifier) @local.definition.variable)
(match_pattern pattern: (identifier) @local.definition.variable)
(test_pattern pattern: (identifier) @local.definition.variable)
(array_pattern (identifier) @local.definition.variable)
(find_pattern (identifier) @local.definition.variable)
(parenthesized_pattern (identifier) @local.definition.variable)
(alternative_pattern (identifier) @local.definition.variable)
(as_pattern (identifier) @local.definition.variable)
(keyword_pattern value: (identifier) @local.definition.variable)
(keyword_pattern key: (hash_key_symbol) @local.definition.variable !value)

; An explicit method call keeps its function highlight even when a local
; variable has the same spelling. Tree-house treats this later helper capture
; as a discard of the generic local-reference candidate above.
(call method: (identifier) @local.ignore)
