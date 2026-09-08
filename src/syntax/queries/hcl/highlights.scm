; SPDX-License-Identifier: MPL-2.0
; Runyte-authored highlighting for tree-sitter-hcl 1.1.0.

(identifier) @variable
(block . (identifier) @keyword)
(attribute . (identifier) @property)
(get_attr (identifier) @property)
(object_elem key: (expression (variable_expr (identifier) @property)))
(function_call . (identifier) @function)

(comment) @comment
(numeric_lit) @number
(bool_lit) @constant.builtin
(null_lit) @constant.builtin

(string_lit) @string
(template_literal) @string
[(quoted_template_start) (quoted_template_end)] @string
[(heredoc_start) (heredoc_identifier)] @label

[
  (template_interpolation_start)
  (template_interpolation_end)
  (template_directive_start)
  (template_directive_end)
  (strip_marker)
] @punctuation.special

["for" "in" "if" "else" "endif" "endfor"] @keyword

[
  "=" "=>" "?" ":" "+" "-" "*" "/" "%" "!"
  "==" "!=" "<" ">" "<=" ">=" "&&" "||"
  ".*" "[*]" (ellipsis)
] @operator

["(" ")" "[" "]" "{" "}"] @punctuation.bracket
["," "."] @punctuation.delimiter
