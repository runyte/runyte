; SPDX-License-Identifier: MIT
; Adapted from tree-sitter-containerfile 0.9.2 queries/injections.scm,
; retaining only Bash shell-command and RUN-heredoc injections.
; Copyright (c) 2026 WharfLab; Copyright (c) 2021 Camden Cheek.
; Parse commands separately so adjacent RUN instructions cannot fuse tokens.
((shell_command) @injection.content
  (#set! injection.language "bash"))

; Exclude the closing delimiter from the shell document.
((run_instruction
  (heredoc_block
    (heredoc_content) @injection.content))
  (#set! injection.language "bash"))
