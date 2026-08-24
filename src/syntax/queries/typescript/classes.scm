; SPDX-License-Identifier: MPL-2.0
; Runyte-authored class addition for tree-sitter-typescript 0.23.2.

(abstract_class_declaration) @class.around
(abstract_class_declaration body: (class_body) @class.inside)
