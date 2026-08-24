; SPDX-License-Identifier: MPL-2.0
; Runyte-authored function query for tree-sitter-rust 0.24.2.

(function_item) @function.around
(function_item body: (block) @function.inside)

; tree-house 0.4 parses included Rust ranges beneath Markdown fences as ERROR
; nodes because the included range starts at a nonzero source byte. Those
; nodes can end before the function body, and a source_file capture would
; over-select a multi-function fence. Mark the layer unsupported instead of
; returning a range that is not truthfully one function.
(source_file
  (ERROR "fn") @function.unsupported)
