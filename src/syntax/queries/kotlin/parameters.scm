; SPDX-License-Identifier: MPL-2.0
; Runyte-authored parameter query for tree-sitter-kotlin-sg 0.4.1.

(function_value_parameters
  [(parameter) (parameter_with_optional_type)] @parameter.inside)

(function_value_parameters
  [(parameter) (parameter_with_optional_type)] @parameter.around
  .
  "," @parameter.around)

(function_value_parameters
  [(parameter) (parameter_with_optional_type)] @parameter.around
  .
  ")")

; The grammar's hidden _function_value_parameter rule exposes a default as
; three siblings: parameter, "=", and the named expression root. Capture each
; range separately so whitespace stays outside the owned text object.
(function_value_parameters
  [(parameter) (parameter_with_optional_type)] @parameter.around
  "=" @parameter.around
  (_) @parameter.around
  .
  "," @parameter.around)

(function_value_parameters
  [(parameter) (parameter_with_optional_type)] @parameter.around
  "=" @parameter.around
  (_) @parameter.around
  .
  ")")

(primary_constructor (class_parameter) @parameter.inside)

(primary_constructor
  (class_parameter) @parameter.around
  .
  "," @parameter.around)

(primary_constructor
  (class_parameter) @parameter.around
  .
  ")")

(lambda_parameters
  [(variable_declaration) (multi_variable_declaration)] @parameter.inside)

(lambda_parameters
  [(variable_declaration) (multi_variable_declaration)] @parameter.around
  .
  "," @parameter.around)

(lambda_parameters
  [(variable_declaration) (multi_variable_declaration)] @parameter.around
  .)

; A typed destructured lambda parameter is likewise flattened into the
; declaration, ":", and its named type root by the hidden _lambda_parameter.
(lambda_parameters
  (multi_variable_declaration) @parameter.around
  ":" @parameter.around
  (_) @parameter.around
  .
  "," @parameter.around)

(lambda_parameters
  (multi_variable_declaration) @parameter.around
  ":" @parameter.around
  (_) @parameter.around
  .)
