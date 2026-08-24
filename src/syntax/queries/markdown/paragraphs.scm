; SPDX-License-Identifier: MPL-2.0
; Runyte-authored paragraph query for tree-sitter-md 0.5.3 block grammar.

(paragraph) @paragraph.around
(paragraph (inline) @paragraph.inside)
