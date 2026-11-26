// SPDX-License-Identifier: MPL-2.0
//! Best-effort YAML mapping source locations, separate from configuration semantics.
use std::collections::BTreeMap;
use tree_house_bindings::{Node, Parser};

pub(crate) fn mapping_locations(source: &str) -> BTreeMap<Vec<String>, usize> {
    let mut result = BTreeMap::new();
    if source.len() > 1024 * 1024 {
        return result;
    }
    let mut parser = Parser::new();
    if parser
        .set_grammar(
            tree_house_bindings::Grammar::try_from(tree_sitter_yaml::LANGUAGE)
                .expect("bundled YAML grammar ABI"),
        )
        .is_err()
    {
        return result;
    }
    let rope = ropey::Rope::from_str(source);
    let Some(tree) =
        parser.parse_with_timeout(rope.slice(..), None, std::time::Duration::from_millis(100))
    else {
        return result;
    };
    let lines = source
        .bytes()
        .enumerate()
        .filter_map(|(i, b)| (b == b'\n').then_some(i))
        .collect::<Vec<_>>();
    fn visit(
        node: Node<'_>,
        source: &str,
        lines: &[usize],
        path: &mut Vec<String>,
        out: &mut BTreeMap<Vec<String>, usize>,
        depth: usize,
    ) {
        if depth > 32 || out.len() >= 131_072 {
            return;
        }
        if matches!(node.kind(), "block_mapping_pair" | "flow_pair") {
            let Some(key) = node.named_child(0) else {
                return;
            };
            let text = &source[key.start_byte() as usize..key.end_byte() as usize];
            let Ok(name) = serde_yaml::from_str::<String>(text) else {
                return;
            };
            path.push(name);
            out.insert(
                path.clone(),
                lines.partition_point(|&i| i < key.start_byte() as usize) + 1,
            );
            if let Some(value) = node.named_child(1) {
                visit(value, source, lines, path, out, depth + 1);
            }
            path.pop();
        } else if !node.kind().contains("sequence") {
            for i in 0..node.named_child_count() {
                if let Some(child) = node.named_child(i) {
                    visit(child, source, lines, path, out, depth + 1);
                }
            }
        }
    }
    visit(
        tree.root_node(),
        source,
        &lines,
        &mut Vec::new(),
        &mut result,
        0,
    );
    result
}
