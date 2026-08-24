; SPDX-License-Identifier: MPL-2.0
; Runyte indentation query for tree-sitter-bash 0.25.1.
[(compound_statement) (do_group) (if_statement) (case_statement) (function_definition) (subshell)] @indent.always
[(array) (expansion)] @indent.begin
