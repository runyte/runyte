; SPDX-License-Identifier: MPL-2.0
; Runyte-authored indentation query for tree-sitter-make 1.1.1. Recipe lines
; require a literal tab rather than the editor's configured indentation unit.
[
  (define_directive)
  (conditional)
] @indent.always
(rule) @indent.tab
