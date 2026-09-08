; SPDX-License-Identifier: MIT
; Adapted from tree-sitter-php 0.24.2 queries/injections-text.scm and
; queries/injections.scm. Omit the unbundled phpdoc grammar so comments
; retain their PHP highlighting. Read the opening label before the body so
; tree-house can resolve its language when the content capture is visited.
; Copyright (c) 2017 Josh Vera, GitHub
; Copyright (c) 2019 Max Brunsfeld, Amaan Qureshi, Christian Frøystad, Caleb White

((text) @injection.content
  (#set! injection.language "html")
  (#set! injection.combined))

((heredoc
  (heredoc_start) @injection.language
  (heredoc_body) @injection.content)
  (#set! injection.include-children))

((nowdoc
  (heredoc_start) @injection.language
  (nowdoc_body) @injection.content)
  (#set! injection.include-children))
