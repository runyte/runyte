; SPDX-License-Identifier: MPL-2.0
; Runyte-authored outline query for tree-sitter-md 0.5.3 block grammar.

(section
  .
  (atx_heading
    (inline) @outline.name)) @outline.heading

(section
  .
  (setext_heading
    (paragraph
      (inline) @outline.name))) @outline.heading
