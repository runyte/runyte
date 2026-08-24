; SPDX-License-Identifier: MPL-2.0
; Runyte-authored section query for tree-sitter-md 0.5.3 block grammar.

(section) @section.around
(section
  .
  [(atx_heading) (setext_heading)]
  (_)* @section.inside)
