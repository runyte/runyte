; SPDX-License-Identifier: MPL-2.0
; Runyte-authored class query for tree-sitter-python 0.25.0.

(class_definition) @class.around
(class_definition body: (block) @class.inside)
