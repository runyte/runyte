; SPDX-License-Identifier: MPL-2.0
; Runyte indentation query for tree-sitter-javascript 0.25.0.
[(statement_block) (class_body) (switch_body)] @indent.always
[(object) (array) (arguments)] @indent.begin
