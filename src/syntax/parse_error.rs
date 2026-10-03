// SPDX-License-Identifier: MPL-2.0

//! Cached parse-error pruning for the pinned Tree-sitter runtime.

use tree_house_bindings::Node;

// tree-house-bindings 0.3.2 does not expose this Tree-sitter accessor. Its
// public Node is repr(C): [u32; 4], two NonNull<c_void> pointers, and a zero-sized
// lifetime marker, matching the bundled runtime's TSNode ABI. Keep this shim
// private to the syntax boundary and recheck that layout on dependency updates.
unsafe extern "C" {
    fn ts_node_has_error(node: Node<'_>) -> bool;
}

pub(super) fn node_has_parse_error(root: Node<'_>) -> bool {
    // SAFETY: root is a live, non-null node from the same tree-house-bindings
    // runtime that supplies this symbol. The repr(C) layout above matches its
    // by-value TSNode parameter. The cached subtree error cost includes both
    // ERROR nodes and parser-inserted missing nodes, with no tree traversal.
    unsafe { ts_node_has_error(root) }
}

pub(super) fn tree_has_parse_error(root: Node<'_>) -> bool {
    if !node_has_parse_error(root.clone()) {
        return false;
    }
    // The native bit also includes hidden grammar recovery nodes that are not
    // visible through Node::child. Preserve the existing diagnostic contract:
    // report only exposed ERROR or missing nodes, skipping clean subtrees.
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        if node.kind() == "ERROR" || node.is_missing() {
            return true;
        }
        stack.extend(
            (0..node.child_count())
                .filter_map(|index| node.child(index))
                .filter(|child| node_has_parse_error(child.clone())),
        );
    }
    false
}
