; SPDX-License-Identifier: MPL-2.0

; Runyte semantic Markdown block highlights for tree-sitter-md 0.5.3.
; The structure follows the upstream query, but its `text.*` captures are
; expressed in the editor's theme vocabulary rather than being discarded.

(atx_heading
  (inline) @markup.heading)

(setext_heading
  (paragraph) @markup.heading)

[
  (atx_h1_marker)
  (atx_h2_marker)
  (atx_h3_marker)
  (atx_h4_marker)
  (atx_h5_marker)
  (atx_h6_marker)
  (setext_h1_underline)
  (setext_h2_underline)
] @markup.heading

[
  (link_title)
  (indented_code_block)
  (fenced_code_block)
] @markup.raw

(fenced_code_block_delimiter) @punctuation.delimiter

; An injected language owns fence content when its info string resolves.
(code_fence_content) @none

(link_destination) @markup.link.url
(link_label) @markup.link.text

[
  (list_marker_plus)
  (list_marker_minus)
  (list_marker_star)
  (list_marker_dot)
  (list_marker_parenthesis)
  (thematic_break)
] @markup.list

[
  (block_continuation)
  (block_quote_marker)
] @markup.quote

(backslash_escape) @string.escape
