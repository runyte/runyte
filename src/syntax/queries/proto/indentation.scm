; SPDX-License-Identifier: MIT
; Adapted from tree-sitter-proto 0.5.0 queries/indents.scm for Runyte's
; bounded indentation capture dialect.
[
  (message_body)
  (enum_body)
  (oneof)
  (service)
  (rpc)
] @indent.always
